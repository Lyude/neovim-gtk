This is a short write-up on the preferred contribution style of this project! Being a rather small
project, we have pretty simple guidelines. Before we go into that though, we must first emphasize
one thing:

**Don't be afraid!**

Being a new contributor to a project can be intimidating, and you may feel like you're "not ready
yet" to contribute. But that's OK, the only way to know the answer to that is to give it your best
shot. It's nearly always more valuable from a maintainer's perspective to teach someone how to
contribute to your project then it is to turn them down.

Now onto the actual contribution guide:

**No AI generated code contributions**

**I just mean code/doc contributions. If you use Claude or what have you for research, answering
questions about this codebase, informational things - that's fine.** I wouldn't recommend it, but
it's not actively harmful so *shrug*.

However, I'm not interested in AI generated code contributions. Code needs to be maintainable and
understood by people, and Claude tends to be pretty bad at writing code anyway. If you need help understanding
something, writing something, etc. - please feel free to ask. You will learn something along the
way, and you'll feel much more accomplished with what you've made.

**Style guidelines**

* Listen to rustfmt mostly, with the following exceptions:
  * Sometimes, particularly when dealing with lots of rendering coordinates, it may be beneficial to
    slightly deviate from rustfmt and instead format things by hand. An example of this can be found
    in src/cursor.rs:

    ```rust
    // …
    if state.anim_phase == AnimPhase::NoFocus {
        #[rustfmt::skip]
        {
            let bg = hl.cursor_bg().to_rgbo(filled_alpha);
            snapshot.append_color(&bg, &Rect::new(          x,           y,   w, 1.0));
            snapshot.append_color(&bg, &Rect::new(          x,           y, 1.0,   h));
            snapshot.append_color(&bg, &Rect::new(          x, y + h - 1.0,   w, 1.0));
            snapshot.append_color(&bg, &Rect::new(x + w - 1.0,           y, 1.0,   h));
        };
        false
    } else {
    // …
    ```

    There's a lot of variables and arithematic happening here, so it's much easier to read if we
    prevent rustfmt from mangling things. Large tables of static data very often fall into this
    category.
  * `gtk::*Builder` usage patterns. We use these all over the place, and the majority of the time
    rustfmt is going to split the method chains here onto separate lines. As a result, single-line
    invocations of Builder types tend to look out of place and are less visually intuitive. So, feel
    free to have rustfmt skip these whenever it tries combining these method chains onto a single
    line. If it's the only Builder invocation in it's scope and it's really, seriously short though,
    feel free to use your best judgement.

* They're guidelines: use your best judgement and don't be afraid of making the wrong decision, if a
  piece of code seems like it'd be much more legible without rustfmt mangling it - feel free to
  throw a `[rustfmt::skip]` onto it. If a maintainer disagrees, they'll just let you know and the
  worst thing you'll have to do is change it ♥.

vim: tw=100 ts=2 sts=2 sw=2 expandtab
