# binage

binage = **bin**ary ma**nager**

A tiny cross-platform manager for portable binaries: single executables and
self-contained application folders.

## Features

- **Binary installation**: Install standalone executables into a centralized directory (`bin/`)
- **Package management**: Install application directories and expose selected executables as commands
- **Cross-platform launchers**: Use Unix symlinks or native Windows `.exe` launchers
- **Conflict prevention**: Refuse to overwrite existing files or commands
- **Safe removal**: Verify file ownership before deleting managed files

## Install

```sh
cargo build --release
# Get `target/release/binage` (Windows: `binage.exe`).
```

or:

```powershell
./scripts/install.ps1
```

## Quick start

```sh
binage init
binage add ./portable-tool
binage ls
```

`init` creates the install root and adds its `bin/` directory to your PATH once.
Open a new terminal afterwards. The root defaults to `~/.local/binage`
(`%LOCALAPPDATA%\binage` on Windows) and is fixed by the first `init`.

## Commands

| Command | Description |
| --- | --- |
| `init [--path DIR]` | Create the install root and configure PATH. `--path` applies to the first init only. |
| `add <FILE>` | Copy a single executable into `bin/`. |
| `add <DIR> -e <FILES>` | Copy a directory into `packages/` and register one command per entry. |
| `ls` | List installed commands. |
| `remove <NAME>` | Remove a command by name. |
| `destruct` | Remove the binage installation, including its managed files and PATH entry. |

## Limitations

- No dependency resolution
- No update or upgrade
