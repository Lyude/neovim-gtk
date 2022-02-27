# neovim-gtk

GTK ui for neovim written in rust using gtk-rs bindings. With
[ligatures](https://github.com/daa84/neovim-gtk/wiki/Configuration#ligatures) support. This is a
fork of the original project from @daa84.

Note that I haven't set up the wiki pages for this repo yet, so wiki links still go to daa84's wiki
repo.

# Screenshot
![Main Window](/screenshots/neovimgtk-screen.png?raw=true)

For more screenshots and description of basic usage see [wiki](https://github.com/daa84/neovim-gtk/wiki/GUI)

# Configuration
To setup font add next line to `ginit.vim`
```vim
call rpcnotify(1, 'Gui', 'Font', 'DejaVu Sans Mono 12')
```
for more details see [wiki](https://github.com/daa84/neovim-gtk/wiki/Configuration)

# Install
## From sources
First check [build prerequisites](#build-prerequisites)

By default to `/usr/local`:
```
make install
```
Or to some custom path:
```
make PREFIX=/some/custom/path install
```

## Fedora
TODO
## Arch Linux
TODO
## openSUSE
TODO
## Windows
TODO

# Build prerequisites
## Linux
First install the GTK development packages. On Debian/Ubuntu derivatives
this can be done as follows:
```shell
apt install libgtk-4-dev
```

On Fedora:
```shell
dnf install atk-devel cairo-devel gdk-pixbuf2-devel glib2-devel gtk4-devel pango-devel
```

Then install the latest rust compiler, best with the
[rustup tool](https://rustup.rs/). The build command:
```
cargo build --release
```

As of writing this (Jan 22, 2022) the packaged rust tools in Fedora also work for building.

## Windows
Neovim-gtk can be compiled using MSYS2 GTK packages. In this case use 'windows-gnu' rust toolchain.
```
SET PKG_CONFIG_PATH=C:\msys64\mingw64\lib\pkgconfig
cargo build --release
```
