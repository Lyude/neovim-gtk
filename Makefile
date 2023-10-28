PREFIX?=/usr/local

test:
	RUST_BACKTRACE=1 cargo test

run:
	RUST_LOG=warn RUST_BACKTRACE=1 cargo run $(CARGO_ARGS) -- --no-fork

install: install-resources
	cargo install $(CARGO_ARGS) --path . --force --root $(DESTDIR)$(PREFIX)

install-debug: install-resources
	cargo install $(CARGO_ARGS) --debug --path . --force --root $(DESTDIR)$(PREFIX)

install-resources:
	mkdir -p $(DESTDIR)$(PREFIX)/share/nvim-gtk/
	cp -r runtime $(DESTDIR)$(PREFIX)/share/nvim-gtk/
	mkdir -p $(DESTDIR)$(PREFIX)/share/applications/
	sed -e "s|Exec=nvim-gtk|Exec=$(PREFIX)/bin/nvim-gtk|" \
		desktop/com.github.Lyude.neovim-gtk.desktop \
		>$(DESTDIR)$(PREFIX)/share/applications/com.github.Lyude.neovim-gtk.desktop
	mkdir -p $(DESTDIR)$(PREFIX)/share/icons/hicolor/128x128/apps/
	cp desktop/com.github.Lyude.neovim-gtk_128.png $(DESTDIR)$(PREFIX)/share/icons/hicolor/128x128/apps/com.github.Lyude.neovim-gtk.png
	mkdir -p $(DESTDIR)$(PREFIX)/share/icons/hicolor/48x48/apps/
	cp desktop/com.github.Lyude.neovim-gtk_48.png $(DESTDIR)$(PREFIX)/share/icons/hicolor/48x48/apps/com.github.Lyude.neovim-gtk.png
	mkdir -p $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/
	cp desktop/com.github.Lyude.neovim-gtk.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/
	mkdir -p $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/
	cp desktop/com.github.Lyude.neovim-gtk-symbolic.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/

.PHONY: all clean test
