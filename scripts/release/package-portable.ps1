param(
    [string]$Executable = "backend/target/release/jolu.exe",
    [string]$OutputDirectory = "backend/target/release/bundle/portable"
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
    throw "缺少免安装版程序：$Executable"
}

# 使用全新临时目录，只收集程序和说明，不包含本机账号、配置或构建缓存。
$staging = Join-Path ([System.IO.Path]::GetTempPath()) ("jolu-portable-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $staging | Out-Null
Copy-Item -LiteralPath $Executable -Destination (Join-Path $staging "Jolu.exe")
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "portable-readme.txt") -Destination (Join-Path $staging "README.txt")
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$archive = Join-Path $OutputDirectory "Jolu-Portable.zip"
Compress-Archive -LiteralPath (Join-Path $staging "Jolu.exe"), (Join-Path $staging "README.txt") -DestinationPath $archive -Force
# 仅删除本次生成的两个文件与空目录，不递归删除任何目录。
Remove-Item -LiteralPath (Join-Path $staging "Jolu.exe"), (Join-Path $staging "README.txt")
[System.IO.Directory]::Delete($staging)
Write-Output "已生成免安装包：$archive"
