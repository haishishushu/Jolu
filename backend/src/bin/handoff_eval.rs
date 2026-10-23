#[cfg(not(debug_assertions))]
compile_error!("handoff_eval 仅允许 debug 构建，不进入安装包");
fn main() {
    #[cfg(debug_assertions)]
    if let Err(error) = jolu_lib::handoff_eval::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
