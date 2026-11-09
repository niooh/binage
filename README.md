# binage

A tiny cross-platform manager for portable binaries: single executables and
self-contained application folders.

## Features

- Install a standalone executable, or a whole directory as a package
- No overwrites: conflicting names abort the command
- Verifies ownership before removing anything
- Unix symlinks, Windows `.cmd` launchers

## Install

```sh
cargo build --release
```

The binary is `target/release/binage` (Windows: `binage.exe`).

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
| `binage init [--path DIR]` | Create the install root and configure PATH. `--path` applies to the first init only. |
| `binage add FILE` | Copy a single executable into `bin/`. The source is left untouched. |
| `binage add DIR -e FILE...` | Copy a directory into `packages/` and register one command per entry. `-e` is required. |
| `binage ls` | List installed commands. |
| `binage remove NAME` | Remove a command by name. |

Command names come from the filename with a trailing `.exe` removed; other
extensions are kept. Entry paths for `add DIR` must be relative and point to
regular files.

`remove` accepts a command name, not a package name. Removing a package's last
entry deletes its directory, and only files created by binage are deleted.

## Limitations

- No dependency resolution
- No update or upgrade
