# appimage-integrator

Integrates AppImages into the desktop by using a watcher and/or a GUI.
Inspired by AppImageLauncher and go-appimage.

## Install

- Arch: PKGBUILDs in [dist/arch](dist/arch) or via the [AUR](https://aur.archlinux.org/packages/appimage-integrator)
- or grab the static binary from the [releases](https://github.com/sh4nks/appimage-integrator/releases)


## Build

```sh
cargo build --release                             # CLI
cargo build --release -p appimage-integrator-gui  # GUI, needs Qt 6
```

## Usage

```sh
appimage-integrator integrate Foo.AppImage  # move to ~/Applications and add to the launcher
appimage-integrator list
appimage-integrator remove <id>             # --keep-file leaves the AppImage in place
appimage-integrator sync                    # integrate new, drop deleted
appimage-integrator watch                   # sync, then keep watching
appimage-integrator update <id>             # or --all, --check
appimage-integrator info Foo.AppImage
```

## Background services

```sh
systemctl --user enable --now appimage-integrator.service       # watch folders
systemctl --user enable --now appimage-integrator-update.timer  # daily update check
```

## Configuration

Optional, in `~/.config/appimage-integrator/config.toml`. See [dist/config.example.toml](dist/config.example.toml).

```toml
appimage_dir = "~/Applications"
watch_dirs = ["~/Applications", "~/Games/AppImages"]
```

## License

MIT
