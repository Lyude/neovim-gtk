# v1.0.3

## Bugs fixed:

* Cargo.lock was out of sync, oops 😳. Will look into setting up a release workflow to prevent this
  from happening in the future. (fixes #58)

---

# v1.0.2

New stable release, bugs fixed:

* OSX (thanks @jacobmischka)
  * #46 - Workaround transparent windows bug in GTK4
  * Disable default shortcuts
* All
  * Fix appearance of error window that displays when initialization fails, which regressed since
    GTK4

## Additional thanks to

* @jacobmischka

---

# v1.0.1

* Update dependencies
* Drop some leftover dead cairo code I missed, and our explicit cairo dependency
* Some bugfixes:
  * Fix funny window sizing issue with long file names
    (https://github.com/Lyude/neovim-gtk/issues/41)
  * Fix scrolling speed on devices like touchpads (https://github.com/Lyude/neovim-gtk/pull/40)
  * Aesthetic improvements to the new underline style (underline should no longer look like it's
    moving under the text cursor)
  * Fix issue with underlines disappearing under the text cursor if no pango item was below the
    cursor

## Additional thanks to

* @jadahl

---

# v1.0.0

We're finally ready to move to GTK4! There's a number of other nice changes that come with this:

- We now use a `gtk::Snapshot` based renderer instead of cairo, and introduce a new `NvimViewport`
  widget that implements the new renderer
- We also convert the `ext_cmdline` over to using the new renderer
- The `ext_popupmenu` popover now uses a `GtkListView` instead of a `GtkTreeView`
- We actually make use of nvim's `flush` event now for screen redraws, which probably should have
  been done from the start. Supporting this means we're dramatically less likely to display screen
  updates to the user before we've finished parsing a full batch of GUI events from nvim
- We also use the `flush` event for popup menu updates, in addition to flattening all of the which
  replaces the previous hacks that were in place to prevent the user from seeing intermediate
  `popup_menu` events. This also allows us to avoid having to use a timed delay for displaying the
  popup menu, which makes things a bit faster :)
- Long taps from touchscreens should register as right clicks now

## Additional thanks to

- @baco
- @jadahl
- Company and the other folks in `#gtk`  who helped a ton with answering questions

---

# v0.4.1

## Bug fixes

- Revert default background type back to dark (#21)

---

# v0.4.0

Note: this is the first version being maintained by Lyude, and as a result I didn't make a thorough
attempt at coming up with a changelog for history that came before me maintaining things (besides
things that were already written in the changelog by @daa84). Therefore, this changelog may be
incomplete. I've also decided to skip v0.3.0 and go directly to v0.4.0, to indicate the difference
in maintenance since things were stuck on v0.3.0 for so long. Future version bumps won't skip
numbers like this.

## Enhancements

- Migration to new ext_linegrid api [ui-linegrid](https://neovim.io/doc/user/ui.html#ui-linegrid)
- New option --cterm-colors [#190](https://github.com/daa84/neovim-gtk/issues/190)
- Migrate to using nvim-rs instead of neovim-lib, this allows us to use async code and handle
  blocking operations far more easily.
- Resize requests are sent immediately vs. intervallically, resulting in much smoother resizing
- We now print RPC request errors as normal neovim error messages
- Closing neovim-gtk is now inhibited during a blocking prompt
- UI elements are now disabled when opening files via the command line, through one of the GUI
  elements, or while neovim-gtk is initializing. This prevents potential RPC request timeouts.
- Don't change nvim directory when changing file browser directory, this behavior wasn't immediately
  obvious and was more confusing then useful.
- Added support for standout highlighting
- Started populating most of the client info for neovim
- Implemented working maps of some neovim arguments which typically hang the GUI if passed directly
  to neovim via `neovim-gtk -- --foo=bar`, including:
  - `-c` (execute command after opening files)
  - `-d` (diff-mode)
- Start using `nvim_input_mouse()`
- Update GTK+ version to 3.26
- Update crates
- Preliminary work for [GTK+4 support](#8):
  - Use `PopoverMenu`s instead of `GtkMenu`s
  - Start using `PopoverMenu` and `Action`s for the file browser
  - Use `Action`s for building the context menu for the drawing area
  - Use a `MenuButton` for the Open button rather than a `Button`
  - Use CSS margins instead of `border_width()` where possible
  - Stop using `size_allocate` events where we can help it
  - Various misc. refactoring
- Use the special color for rendering underline
- Add support for the `:cq` command (#15, @bk2204)
- Improve algorithm for determining popup menu column sizes
- Update GTK+ tabling visibility on tabline option changes
- Make info in the completion popup scrollable

## Bug fixes

- `VimLeavePre` and `VimLeave` are now correctly executed upon exiting
- `E365 ("File already opened in another process")` prompts no longer hang when opening files via
  the command line
- The runtime path for our various vim scripts is now correctly set when using `cargo run`
- Resizing while neovim is stuck on a blocking prompt no longer hangs
- Focus changes while neovim is stuck on a blocking prompt no longer hang
- Use the special color for rendering underlines and undercurls when it's explicitly set, otherwise
  fallback to the foreground color (except for undercurls, where we default to red in this case).
  (#10)
- Fix issues with various unicode graphemes being misaligned when rendered using certain fonts (#7,
  #5, @medicalwei)
- Fix crashes and most rendering issues when rendering combining glyphs that require a fallback font
  to be used
- Round up background cell width (#1, @jacobmischka)
- Silently ignore the blend attribute for highlights, at least until we've added support for it
  (#17, @bk2204).
- Don't use predictably named temporary files (#20, @bk2204)
- Fix undercurl rendering with certain fonts (#11)
- Stop completion popups from changing colors changing when they shouldn't be
- Fix GTK+ tabline visibility issues when trying to disable the external tabline
- Fix undercurl rendering for double width graphemes under the cursor
- Fix coloring with respect to the `background` option in neovim when either one or both of the
  foreground and background colors are missing.

## Special thanks to those who contributed patches this release

- @medicalwei
- @bk2204
- @jacobmischka

<!-- vim: tw=100 colorcolumn=100 ts=2 sts=2 sw=2 expandtab
-->
