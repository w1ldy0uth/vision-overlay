# Сборка и проверки

Среда: Windows x64, Git, Rust/MSVC 1.99.1, Visual Studio C++ Build Tools,
Node.js 24.22.1. Для тестов — Python 3.14 и requirements-dev.txt. Rust закреплён
в rust-toolchain.toml, зависимости — в Cargo.lock и web/package-lock.json.

```powershell
git clone https://github.com/w1ldy0uth/vision-overlay.git
cd vision-overlay
powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File ./scripts/Build-Portable.ps1
```

Скрипт проверяет и распаковывает packaging/catalog/bootstrap.visioncatalog.gz,
запускает Rust-валидатор полного справочника, формирует console-reference файлы,
собирает frontend и встраивает его в сервер. Затем собирает release EXE,
проверяет официальный Python runtime по закреплённому SHA256, собирает лицензии,
ZIP и контрольные суммы. Рабочая установка и личная БД не нужны. При первой
сборке нужен интернет для Rust/npm/Python-зависимостей.

Для другого справочника передайте -CatalogBundle <пакет.visioncatalog>.
Неполный/повреждённый пакет отвергается. Происхождение и хеши источников
начального справочника записаны в packaging/catalog/source.json.

Результат: dist/VisionOverlay-<версия>-windows-x64-portable.zip.
Версия определяется workspace.package.version; -Version может только подтвердить
то же значение. Output не перезаписывается: для повторной сборки используйте
другой чистый checkout или сохраните предыдущий output отдельно.

## Проверки

```powershell
python -m pip install -r requirements-dev.txt
python scripts/audit_publication.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test -p auto-artifactarium --lib --locked
python -m unittest discover -s scripts -p "test_*.py"
python scripts/check_contract_examples.py
npm --prefix web test
powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File scripts/Test-AutoCapturePolicy.ps1
python scripts/verify_portable.py dist/VisionOverlay-<версия>-windows-x64-portable.zip --output local-data/portable-check-new
```

Portable-проверка сверяет ZIP/хеши, embedded Python и demo probe, запускает
приложение с пустой БД, проверяет полный справочник и gi.json, повторный экземпляр,
shutdown, перенос в путь с кириллицей/пробелами. Игра, реальный захват и задачи
Windows не запускаются. На CI используется --headless без зависимости от Explorer/трея.

## Debug

```powershell
powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File scripts/Build-Web.ps1
cargo build --workspace --locked
./target/debug/vision-server.exe --initialize
```

Debug по умолчанию использует local-data и порт 18788; portable — data относительно
EXE и порт 18789. Игнорируемая docs содержит рабочую историю, публичные инструкции — guides.

## Релиз

CI проверяет код, собирает portable и проверяет архив. Push тега, совпадающего
с версией (v0.1.0-beta.1), запускает ту же проверку и отдельный job публикации.
Он получает проверенный артефакт и создаёт prerelease. Build job имеет contents: read,
publish job — contents: write только для тегов. Actions закреплены на полные SHA.
GitHub runner с инструментами сборки не заменяет проверку чистой Windows с игрой;
для первой беты она явно отложена.
