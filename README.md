# neovim-gtk

<a href="https://repology.org/project/neovim-gtk/versions">
    <img src="https://repology.org/badge/vertical-allrepos/neovim-gtk.svg" alt="Packaging status" align="right">
</a>

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

## Fedora
TODO

## Nix
On Nix/Home-Manager/NixOS, `neovim-gtk` can be installed from `nixpkgs`:

```ShellSession
$ nix-env -iA nixos.neovim-gtk # On NixOS, without flakes
$ nix-env -iA nixpkgs.neovim-gtk # Not on NixOS, without flakes
$ nix profile install nixpkgs#neovim-gtk # with flakes
```

Or, add to your NixOS configuration:

```nix
{ pkgs, ... }: {
  environment.systemPackages = with pkgs; [ neovim-gtk ];
}
```

## Arch Linux
On Arch Linux, `neovim-gtk` can be installed from the AUR, here with `yay`:

```ShellSession
$ yay -S neovim-gtk
$ # or, latest commit on main:
$ yay -S neovim-gtk-git
```

## openSUSE

```ShellSession
$ zypper install neovim-gtk
```

## Windows
TODO

## FreeBSD
On FreeBSD, `neovim-gtk` can be installed either as a port or with `pkg`:

```ShellSession
$ # from source
$ cd /usr/ports/editors/neovim-gtk/ && make install clean
$ # prebuilt
$ pkg install editors/neovim-gtk
```

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
