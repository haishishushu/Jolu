[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$BackupPath,
    [Parameter(Mandatory = $true)][string]$DestinationPath
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$backup = [IO.Path]::GetFullPath($BackupPath)
$destination = [IO.Path]::GetFullPath($DestinationPath)
$name = [IO.Path]::GetFileName($backup)
if ($name -notmatch '^(?<original>.+)\.\d+\.bak\.dpapi$') {
    throw '请选择 Jolu 生成的 .bak.dpapi 备份文件。'
}
if ([IO.File]::Exists($destination) -or [IO.Directory]::Exists($destination)) {
    throw '目标已存在；为避免覆盖配置，请选择新的恢复文件路径。'
}
$entropy = [Text.Encoding]::UTF8.GetBytes($Matches['original'])
$encrypted = [IO.File]::ReadAllBytes($backup)
$plain = $null
$stream = $null
try {
    $plain = [Security.Cryptography.ProtectedData]::Unprotect(
        $encrypted, $entropy, [Security.Cryptography.DataProtectionScope]::CurrentUser
    )
    $stream = [IO.File]::Open($destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    $stream.Write($plain, 0, $plain.Length)
    $stream.Flush($true)
} catch {
    if ($null -ne $stream) {
        $stream.Dispose()
        $stream = $null
        [IO.File]::Delete($destination)
    }
    throw '备份恢复失败。请使用创建备份时的 Windows 用户，并确认目标目录存在且可写。原 CLI 文件未修改。'
} finally {
    if ($null -ne $stream) { $stream.Dispose() }
    if ($null -ne $plain) { [Array]::Clear($plain, 0, $plain.Length) }
}
Write-Output "已恢复到新文件：$destination；原 CLI 文件未修改。"
