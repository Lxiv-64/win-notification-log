<#
.SYNOPSIS
  Сборка нативной версии Win Notification Log и подготовка готовых к запуску файлов.

.DESCRIPTION
  Выполняет тесты, собирает release, копирует wnl.exe (приложение) и wnl-cli.exe
  (консольная диагностика) в rust\dist.  Артефакты компилятора при этом размещаются
  в профиле пользователя (см. ~\.cargo\config.toml, ключ build.target-dir), поэтому
  каталог проекта не раздувается.

.EXAMPLE
  .\rust\build.ps1
  .\rust\build.ps1 -Run
  .\rust\build.ps1 -SkipTests -Run
#>
[CmdletBinding()]
param(
    [switch]$Run,
    [switch]$SkipTests
)

# Внешние инструменты (cargo) пишут ход сборки в stderr, поэтому 'Stop' превращал бы
# обычный вывод в исключение. Успех проверяем по $LASTEXITCODE.
$ErrorActionPreference = 'Continue'
# Готовые файлы складываем рядом с проектом: rust\dist.
$dist = Join-Path $PSScriptRoot 'dist'

# Cargo устанавливается в профиль пользователя; добавляем его в PATH, если нужно.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if ((Test-Path $cargoBin) -and ($env:Path -notlike "*$cargoBin*")) {
    $env:Path = "$cargoBin;$env:Path"
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo не найден. Установите Rust: https://rustup.rs (см. rust\README.md)."
}

# Через 'cmd /c … 2>&1' вывод cargo приходит обычным текстом: PowerShell перестаёт
# разбирать сообщения сборщика как ошибки, а код возврата остаётся в $LASTEXITCODE.
function Invoke-Cargo {
    param([string]$CommandLine)
    $output = & cmd /c "$CommandLine 2>&1"
    $code = $LASTEXITCODE
    $output | ForEach-Object { Write-Host $_ }
    return $code
}

# Готовые файлы не должны нести путь машины, на которой они собраны: Rust вшивает
# абсолютные пути исходников в сообщения паник.  Переназначения ниже заменяют их
# на нейтральные, поэтому собранный кем угодно exe ничем не связан с его автором.
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$env:RUSTFLAGS = @(
    "--remap-path-prefix=$PSScriptRoot=rust"
    "--remap-path-prefix=$projectRoot=win-notification-log"
    "--remap-path-prefix=$env:USERPROFILE\.cargo=cargo"
    "--remap-path-prefix=$env:USERPROFILE\.rustup=rustup"
) -join ' '

Push-Location $PSScriptRoot
try {
    if (-not $SkipTests) {
        Write-Host '== cargo test ==' -ForegroundColor Cyan
        if ((Invoke-Cargo 'cargo test') -ne 0) { throw 'тесты не прошли' }
    } else {
        Write-Host 'тесты пропущены (-SkipTests)' -ForegroundColor DarkGray
    }

    Write-Host '== cargo build --release ==' -ForegroundColor Cyan
    if ((Invoke-Cargo 'cargo build --release') -ne 0) { throw 'сборка не удалась' }

    # Каталог артефактов спрашиваем у самого cargo, чтобы уважать target-dir из конфигурации.
    $metadata = & cmd /c 'cargo metadata --format-version 1 --no-deps 2>NUL' | ConvertFrom-Json
    $release = Join-Path $metadata.target_directory 'release'

    New-Item -ItemType Directory -Path $dist -Force | Out-Null
    $produced = @()
    foreach ($name in 'wnl.exe', 'wnl-cli.exe') {
        $source = Join-Path $release $name
        if (-not (Test-Path $source)) { throw "не найден результат сборки: $source" }
        $target = Join-Path $dist $name
        try {
            Copy-Item $source $target -Force -ErrorAction Stop
        } catch {
            # Windows не даёт заменить запущенный файл: это ожидаемая ситуация,
            # когда приложение осталось в трее, и подсказка важнее трассировки.
            Write-Host ('не удалось обновить {0}: файл занят запущенным приложением' -f $name) -ForegroundColor Yellow
            Write-Host 'закройте «Win Notification Log» (меню трея → Выход) и повторите сборку' -ForegroundColor Yellow
            throw 'готовые файлы не обновлены'
        }
        $produced += Get-Item $target
    }
} finally {
    Pop-Location
}

Write-Host ('артефакты сборки: {0}' -f $release) -ForegroundColor DarkGray
Write-Host '== готово ==' -ForegroundColor Green
$produced | ForEach-Object { Write-Host ('  {0}  {1:N0} байт' -f $_.FullName, $_.Length) }
Write-Host ('Запуск приложения: {0}\wnl.exe' -f $dist) -ForegroundColor Green

if ($Run) {
    Start-Process -FilePath (Join-Path $dist 'wnl.exe')
}
