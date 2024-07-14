# neovim-gtk

[![CI](https://github.com/Lyude/neovim-gtk/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Lyude/neovim-gtk/actions/workflows/ci.yml)

GTK ui for neovim written in rust using gtk-rs bindings. With
[ligatures](https://github.com/daa84/neovim-gtk/wiki/Configuration#ligatures) support. This project
began as a fork of @daa84's neovim-gtk.

There are a very large number of improvements from @daa84's version, including:

* Lots of bugfixes
* We're fully ported to GTK4, and have a Snapshot based renderer instead of a cairo based renderer
* _Smooth_ resizing

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
## Ubuntu (and probably Debian)

Install nvim, prerequisites, Rust + Cargo.

* Use rustup.rs -- https://rustup.rs
* This example has to be done as root since the rustup installer installs cargo in one's home dir

Example (Tested on Ubuntu 23.04)

``` shell
#become root
sudo su -

#install prereq (see below)
apt update
apt install neovim
apt install libgtk-4-dev -y

#install rust+cargo
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

#at this point, cargo is installed, but not in your $PATH
#quit this root shell and spawn a new one or source your profile to get cargo in your $PATH
exit
sudo su -

#Clone this repo, make, make install
git clone https://github.com/Lyude/neovim-gtk.git
cd neovim-gtk/
make
make install

# exit root
exit

#run nvim-gtk as a user
nvim-gtk

#or, if you installed neovim from source, make sure nvim-gtk knows where nvim is (e.g. /opt/nvim/bin/nvim)
nvim-gtk --nvim-bin-path=/opt/nvim/bin/nvim
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
``` shell
apt install libgtk-4-dev
```

On Fedora:
```bash
dnf install atk-devel glib2-devel pango-devel gtk4-devel
```

Then install the latest rust compiler, best with the
[rustup tool](https://rustup.rs/). The build command:
```
cargo build --release
```

As of writing this (Dec 16, 2022) the packaged rust tools in Fedora also work for building.

## Windows
Neovim-gtk can be compiled using MSYS2 GTK packages. In this case use 'windows-gnu' rust toolchain.
```
SET PKG_CONFIG_PATH=C:\msys64\mingw64\lib\pkgconfig
cargo build --release
```
