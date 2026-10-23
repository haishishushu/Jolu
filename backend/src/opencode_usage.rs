//! OpenCode 统计白名单：消息正文和凭证不进入 Jolu。
use crate::{db::RequestRecord, source_store::Record};
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum OpenCodeLayout { JsonFiles, SqliteV1, SqliteV2 }
pub fn detect_layout(db:&Connection)->Result<OpenCodeLayout,String>{
    for (table,kind) in [("session_message",OpenCodeLayout::SqliteV2),("message",OpenCodeLayout::SqliteV1)] {
        if db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",[table],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {return Ok(kind)}
    }
    Err("OpenCode 数据库结构不受支持，未采集".into())
}
fn parse(v:&Value,id:&str,session:&str,ts:i64)->Option<Record>{
    if v.get("role").or_else(||v.get("type")).and_then(Value::as_str).is_some_and(|s|s!="assistant"){return None}
    let tokens=v.get("tokens")?;if !tokens.is_object(){return None}
    let n=|key:&str|tokens.pointer(key).and_then(Value::as_i64).filter(|v|*v>=0);
    let input=n("/input");let visible=n("/output");let reasoning=n("/reasoning");
    let output=visible.zip(reasoning).and_then(|(v,r)|v.checked_add(r));let read=n("/cache/read");let write=n("/cache/write");
    if [input,visible,reasoning,read,write,n("/total")].iter().all(Option::is_none){return None}
    // 持久化 TokenUsage.output 是可见输出；上游 AI.Usage.outputTokens 才包含 reasoning。
    // 来源显式 total 优先；否则缺失字段保持未知，不用零补齐。
    let total=n("/total").or_else(||[input,visible,reasoning,read,write].into_iter().try_fold(0i64,|a,b|a.checked_add(b?)));
    let provider=v.get("providerID").or_else(||v.pointer("/model/providerID")).and_then(Value::as_str).unwrap_or("unknown");
    Some(Record {request:RequestRecord{platform:"opencode".into(),source:format!("native:opencode:{provider}"),dedup_key:format!("{session}:{id}"),session_id:Some(format!("opencode:{session}")),ts,
        model:v.get("modelID").or_else(||v.pointer("/model/id")).and_then(Value::as_str).map(str::to_owned),input_tokens:input,output_tokens:output,cache_read_tokens:read,cache_write_tokens:write,total_tokens:total,effort:None},
        input_semantics:"excludes_cache",credits:None,original_credits:None,billable:None,context_ratio:None,reasoning:n("/reasoning"),tool:None})
}
pub fn read_usage(db:&Connection,layout:OpenCodeLayout,since_ms:i64)->Result<Vec<Record>,String>{
    let table=match layout{OpenCodeLayout::SqliteV1=>"message",OpenCodeLayout::SqliteV2=>"session_message",OpenCodeLayout::JsonFiles=>return Err("JSON 来源需按文件读取".into())};
    let mut q=db.prepare(&format!("SELECT id,session_id,time_created,data FROM {table} WHERE time_created>=?1")).map_err(|_|"OpenCode 消息表结构不受支持".to_string())?;
    let rows=q.query_map([since_ms],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).map_err(|e|e.to_string())?;
    let mut out=vec![];for row in rows{let(id,session,ts,data)=row.map_err(|e|e.to_string())?;let value=serde_json::from_str(&data).map_err(|_|"OpenCode 消息数据无效")?;if let Some(record)=parse(&value,&id,&session,ts){out.push(record)}}
    if layout==OpenCodeLayout::SqliteV2 && db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='message')",[],|r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
        let newer:std::collections::HashSet<_>=out.iter().map(|r|r.request.dedup_key.clone()).collect();
        out.extend(read_usage(db,OpenCodeLayout::SqliteV1,since_ms)?.into_iter().filter(|r|!newer.contains(&r.request.dedup_key)));
    }
    Ok(out)
}
pub fn read_json_files(root:&Path)->Result<Vec<Record>,String>{
    fn visit(path:&Path,out:&mut Vec<Record>,depth:usize)->Result<(),String>{
        if depth>5{return Err("OpenCode 文件目录层级异常".into())}
        for entry in std::fs::read_dir(path).map_err(|e|e.to_string())?{let entry=entry.map_err(|e|e.to_string())?;let ty=entry.file_type().map_err(|e|e.to_string())?;if ty.is_symlink(){continue}let p=entry.path();if ty.is_dir(){visit(&p,out,depth+1)?}else if p.extension().is_some_and(|e|e=="json"){
            if entry.metadata().map_err(|e|e.to_string())?.len()>16*1024*1024{return Err("OpenCode 消息文件过大".into())}
            let v:Value=serde_json::from_str(&std::fs::read_to_string(&p).map_err(|e|e.to_string())?).map_err(|_|"OpenCode 消息 JSON 无效")?;
            if let (Some(id),Some(session),Some(ts))=(v.get("id").and_then(Value::as_str),v.get("sessionID").and_then(Value::as_str),v.pointer("/time/created").and_then(Value::as_i64)){if let Some(record)=parse(&v,id,session,ts){out.push(record)}}
        }}Ok(())
    }
    let _layout=OpenCodeLayout::JsonFiles;let mut out=vec![];visit(root,&mut out,0)?;Ok(out)
}
#[cfg(test)] mod tests{
    use super::*;
    #[test] fn missing_reasoning_is_not_a_known_zero(){
        let r=parse(&serde_json::json!({"role":"assistant","tokens":{"output":30,"total":30}}),"m","s",1).unwrap();
        assert_eq!(r.request.output_tokens,None);assert_eq!(r.request.total_tokens,Some(30));assert_eq!(r.reasoning,None);
    }
    #[test] fn coexistence_preserves_v1_and_prefers_v2_duplicates(){
        let db=Connection::open_in_memory().unwrap();
        for table in ["message","session_message"] {db.execute_batch(&format!("CREATE TABLE {table}(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);")).unwrap();}
        let data=r#"{"role":"assistant","tokens":{"input":10,"output":5,"cache":{"read":0,"write":0}}}"#;
        db.execute("INSERT INTO message VALUES('m','s',123,?1)",[data]).unwrap();
        assert_eq!(read_usage(&db,detect_layout(&db).unwrap(),0).unwrap().len(),1);
        db.execute("INSERT INTO session_message VALUES('m','s',123,?1)",[data]).unwrap();
        assert_eq!(read_usage(&db,detect_layout(&db).unwrap(),0).unwrap().len(),1);
    }
    #[test] #[ignore="鼠鼠授权后的本机OpenCode只读验证"]
    fn local_opencode_read_only(){
        let path=crate::opencode::data_dir(&crate::creds::home().unwrap()).join("opencode.db");
        let db=Connection::open_with_flags(path,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let rows=read_usage(&db,detect_layout(&db).unwrap(),0).unwrap();
        println!("OpenCode usage rows={}, total_tokens={}",rows.len(),rows.iter().filter_map(|r|r.request.total_tokens).sum::<i64>());
        assert!(!rows.is_empty());
        let dest=crate::db::open(Path::new(":memory:")).unwrap();
        assert_eq!(crate::source_store::write(&dest,&rows).unwrap(),rows.len());
        assert_eq!(crate::source_store::write(&dest,&rows).unwrap(),0);
        let total:i64=dest.query_row("SELECT sum(total_tokens) FROM requests",[],|r|r.get(0)).unwrap();
        assert_eq!(total,rows.iter().filter_map(|r|r.request.total_tokens).sum::<i64>());
    }
    #[test] fn layouts_and_token_semantics(){for (table,layout) in [("message",OpenCodeLayout::SqliteV1),("session_message",OpenCodeLayout::SqliteV2)]{let db=Connection::open_in_memory().unwrap();db.execute_batch(&format!("CREATE TABLE {table}(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);")).unwrap();db.execute(&format!("INSERT INTO {table} VALUES('m','s',123,?1)"),[r#"{"role":"assistant","tokens":{"input":100,"output":30,"reasoning":5,"cache":{"read":20,"write":0}}}"#]).unwrap();assert_eq!(detect_layout(&db).unwrap(),layout);let v=read_usage(&db,layout,0).unwrap();assert_eq!(v[0].request.output_tokens,Some(35));assert_eq!(v[0].request.total_tokens,Some(155));assert!(read_usage(&db,layout,124).unwrap().is_empty());}}
    #[test] fn missing_tokens_are_unknown(){assert!(parse(&serde_json::json!({"role":"assistant"}),"m","s",1).is_none());assert!(detect_layout(&Connection::open_in_memory().unwrap()).is_err());}
}
