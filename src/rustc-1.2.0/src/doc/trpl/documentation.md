% Documentation

Documentation is an important part of any software project, and it's
first-class in Rust. Let's talk about the tooling Rust gives you to
document your project.

## About `rustdoc`

The Rust distribution includes a tool, `rustdoc`, that generates documentation.
`rustdoc` is also used by Cargo through `cargo doc`.

Documentation can be generated in two ways: from source code, and from
standalone Markdown files.

## Documenting source code

The primary way of documenting a Rust project is through annotating the source
code. You can use documentation comments for this purpose:

```rust,ignore
/// Constructs a new `Rc<T>`.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
///
/// let five = Rc::new(5);
/// ```
pub fn new(value: T) -> Rc<T> {
    // implementation goes here
}
```

This code generates documentation that looks [like this][rc-new]. I've left the
implementation out, with a regular comment in its place. That's the first thing
to notice about this annotation: it uses `///`, instead of `//`. The triple slash
indicates a documentation comment.

Documentation comments are written in Markdown.

Rust keeps track of these comments, and uses them when generating
documentation. This is important when documenting things like enums:

```rust
/// The `Option` type. See [the module level documentation](../) for more.
enum Option<T> {
    /// No value
    None,
    /// Some value `T`
    Some(T),
}
```

The above works, but this does not:

```rust,ignore
/// The `Option` type. See [the module level documentation](../) for more.
enum Option<T> {
    None, /// No value
    Some(T), /// Some value `T`
}
```

You'll get an error:

```text
hello.rs:4:1: 4:2 error: expected ident, found `}`
hello.rs:4 }
           ^
```

This [unfortunate error](https://github.com/rust-lang/rust/issues/22547) is
correct: documentation comments apply to the thing after them, and there's no
thing after that last comment.

[rc-new]: http://doc.rust-lang.org/nightly/std/rc/struct.Rc.html#method.new

### Writing documentation comments

Anyway, let's cover each part of this comment in detail:

```rust
/// Constructs a new `Rc<T>`.
# fn foo() {}
```

The first line of a documentation comment should be a short summary of its
functionality. One sentence. Just the basics. High level.

```rust
///
/// Other details about constructing `Rc<T>`s, maybe describing complicated
/// semantics, maybe additional options, all kinds of stuff.
///
# fn foo() {}
```

Our original example had just a summary line, but if we had more things to say,
we could have added more explanation in a new paragraph.

#### Special sections

Next, are special sections. These are indicated with a header, `#`. There
are three kinds of headers that are commonly used. They aren't special syntax,
just convention, for now.

```rust
/// # Panics
# fn foo() {}
```

Unrecoverable misuses of a function (i.e. programming errors) in Rust are
usually indicated by panics, which kill the whole current thread at the very
least. If your function has a non-trivial contract like this, that is
detected/enforced by panics, documenting it is very important.

```rust
/// # Failures
# fn foo() {}
```

If your function or method returns a `Result<T, E>`, then describing the
conditions under which it returns `Err(E)` is a nice thing to do. This is
slightly less important than `Panics`, because failure is encoded into the type
system, but it's still a good thing to do.

```rust
/// # Safety
# fn foo() {}
```

If your function is `unsafe`, you should explain which invariants the caller is
responsible for upholding.

```rust
/// # Examples
///
/// ```
/// use std::rc::Rc;
///
/// let five = Rc::new(5);
/// ```
# fn foo() {}
```

Third, `Examples`. Include one or more examples of using your function or
method, and your users will love you for it. These examples go inside of
code block annotations, which we'll talk about in a moment, and can have
more than one section:

```rust
/// # Examples
///
/// Simple `&str` patterns:
///
/// ```
/// let v: Vec<&str> = "Mary had a little lamb".split(' ').collect();
/// assert_eq!(v, vec!["Mary", "had", "a", "little", "lamb"]);
/// ```
///
/// More complex patterns with a lambda:
///
/// ```
/// let v: Vec<&str> = "abc1def2ghi".split(|c: char| c.is_numeric()).collect();
/// assert_eq!(v, vec!["abc", "def", "ghi"]);
/// ```
# fn foo() {}
```

Let's discuss the details of these code blocks.

#### Code block annotations

To write some Rust code in a comment, use the triple graves:

```rust
/// ```
/// println!("Hello, world");
/// ```
# fn foo() {}
```

If you want something that's not Rust code, you can add an annotation:

```rust
/// ```c
/// printf("Hello, world\n");
/// ```
# fn foo() {}
```

This will highlight according to whatever language you're showing off.
If you're just showing plain text, choose `text`.

It's important to choose the correct annotation here, because `rustdoc` uses it
in an interesting way: It can be used to actually test your examples, so that
they don't get out of date. If you have some C code but `rustdoc` thinks it's
Rust because you left off the annotation, `rustdoc` will complain when trying to
generate the documentation.

## Documentation as tests

Let's discuss our sample example documentation:

```rust
/// ```
/// println!("Hello, world");
/// ```
# fn foo() {}
```

You'll notice that you don't need a `fn main()` or anything here. `rustdoc` will
automatically add a main() wrapper around your code, and in the right place.
For example:

```rust
/// ```
/// use std::rc::Rc;
///
/// let five = Rc::new(5);
/// ```
# fn foo() {}
```

This will end up testing:

```rust
fn main() {
    use std::rc::Rc;
    let five = Rc::new(5);
}
```

Here's the full algorithm rustdoc uses to postprocess examples:

1. Any leading `#![foo]` attributes are left intact as crate attributes.
2. Some common `allow` attributes are inserted, including
   `unused_variables`, `unused_assignments`, `unused_mut`,
   `unused_attributes`, and `dead_code`. Small examples often trigger
   these lints.
3. If the example does not contain `extern crate`, then `extern crate
   <mycrate>;` is inserted.
2. Finally, if the example does not contain `fn main`, the remainder of the
   text is wrapped in `fn main() { your_code }`

Sometimes, this isn't enough, though. For example, all of these code samples
with `///` we've been talking about? The raw text:

```text
/// Some documentation.
# fn foo() {}
```

looks different than the output:

```rust
/// Some documentation.
# fn foo() {}
```

Yes, that's right: you can add lines that start with `# `, and they will
be hidden from the output, but will be used when compiling your code. You
can use this to your advantage. In this case, documentation comments need
to apply to some kind of function, so if I want to show you just a
documentation comment, I need to add a little function definition below
it. At the same time, it's just there to satisfy the compiler, so hiding
it makes the example more clear. You can use this technique to explain
longer examples in detail, while still preserving the testability of your
documentation. For example, this code:

```rust
let x = 5;
let y = 6;
println!("{}", x + y);
```

Here's an explanation, rendered:

First, we set `x` to five:

```rust
let x = 5;
# let y = 6;
# println!("{}", x + y);
```

Next, we set `y` to six:

```rust
# let x = 5;
let y = 6;
# println!("{}", x + y);
```

Finally, we print the sum of `x` and `y`:

```rust
# let x = 5;
# let y = 6;
println!("{}", x + y);
```

Here's the same explanation, in raw text:

> First, we set `x` to five:
>
> ```text
> let x = 5;
> # let y = 6;
> # println!("{}", x + y);
> ```
>
> Next, we set `y` to six:
>
> ```text
> # let x = 5;
> let y = 6;
> # println!("{}", x + y);
> ```
>
> Finally, we print the sum of `x` and `y`:
>
> ```text
> # let x = 5;
> # let y = 6;
> println!("{}", x + y);
> ```

By repeating all parts of the example, you can ensure that your example still
compiles, while only showing the parts that are relevant to that part of your
explanation.

### Documenting macros

Here’s an example of documenting a macro:

```rust
/// Panic with a given message unless an expression evaluates to true.
///
/// # Examples
///
/// ```
/// # #[macro_use] extern crate foo;
/// # fn main() {
/// panic_unless!(1 + 1 == 2, “Math is broken.”);
/// # }
/// ```
///
/// ```should_panic
/// # #[macro_use] extern crate foo;
/// # fn main() {
/// panic_unless!(true == false, “I’m broken.”);
/// # }
/// ```
#[macro_export]
macro_rules! panic_unless {
    ($condition:expr, $($rest:expr),+) => ({ if ! $condition { panic!($($rest),+); } });
}
# fn main() {}
```

You’ll note three things: we need to add our own `extern crate` line, so that
we can add the `#[macro_use]` attribute. Second, we’ll need to add our own
`main()` as well. Finally, a judicious use of `#` to comment out those two
things, so they don’t show up in the output.

### Running documentation tests

To run the tests, either

```bash
$ rustdoc --test path/to/my/crate/root.rs
# or
$ cargo test
```

That's right, `cargo test` tests embedded documentation too. However, 
`cargo test` will not test binary crates, only library ones. This is
due to the way `rustdoc` works: it links against the library to be tested,
but with a binary, there’s nothing to link to.

There are a few more annotations that are useful to help `rustdoc` do the right
thing when testing your code:

```rust
/// ```ignore
/// fn foo() {
/// ```
# fn foo() {}
```

The `ignore` directive tells Rust to ignore your code. This is almost never
what you want, as it's the most generic. Instead, consider annotating it
with `text` if it's not code, or using `#`s to get a working example that
only shows the part you care about.

```rust
/// ```should_panic
/// assert!(false);
/// ```
# fn foo() {}
```

`should_panic` tells `rustdoc` that the code should compile correctly, but
not actually pass as a test.

```rust
/// ```no_run
/// loop {
///     println!("Hello, world");
/// }
/// ```
# fn foo() {}
```

The `no_run` attribute will compile your code, but not run it. This is
important for examples such as "Here's how to start up a network service,"
which you would want to make sure compile, but might run in an infinite loop!

### Documenting modules

Rust has another kind of doc comment, `//!`. This comment doesn't document the next item, but the enclosing item. In other words:

```rust
mod foo {
    //! This is documentation for the `foo` module.
    //!
    //! # Examples

    // ...
}
```

This is where you'll see `//!` used most often: for module documentation. If
you have a module in `foo.rs`, you'll often open its code and see this:

```rust
//! A module for using `foo`s.
//!
//! The `foo` module contains a lot of useful functionality blah blah blah
```

### Documentation comment style

Check out [RFC 505][rfc505] for full conventions around the style and format of
documentation.

[rfc505]: https://github.com/rust-lang/rfcs/blob/master/text/0505-api-comment-conventions.md

## Other documentation

All of this behavior works in non-Rust source files too. Because comments
are written in Markdown, they're often `.md` files.

When you write documentation in Markdown files, you don't need to prefix
the documentation with comments. For example:

```rust
/// # Examples
///
/// ```
/// use std::rc::Rc;
///
/// let five = Rc::new(5);
/// ```
# fn foo() {}
```

is just

~~~markdown
# Examples

```
use std::rc::Rc;

let five = Rc::new(5);
```
~~~

when it's in a Markdown file. There is one wrinkle though: Markdown files need
to have a title like this:

```markdown
% The title

This is the example documentation.
```

This `%` line needs to be the very first line of the file.

## `doc` attributes

At a deeper level, documentation comments are sugar for documentation attributes:

```rust
/// this
# fn foo() {}

#[doc="this"]
# fn bar() {}
```

are the same, as are these:

```rust
//! this

#![doc="/// this"]
```

You won't often see this attribute used for writing documentation, but it
can be useful when changing some options, or when writing a macro.

### Re-exports

`rustdoc` will show the documentation for a public re-export in both places:

```ignore
extern crate foo;

pub use foo::bar;
```

This will create documentation for bar both inside the documentation for the
crate `foo`, as well as the documentation for your crate. It will use the same
documentation in both places.

This behavior can be suppressed with `no_inline`:

```ignore
extern crate foo;

#[doc(no_inline)]
pub use foo::bar;
```

### Controlling HTML

You can control a few aspects of the HTML that `rustdoc` generates through the
`#![doc]` version of the attribute:

```rust
#![doc(html_logo_url = "http://www.rust-lang.org/logos/rust-logo-128x128-blk-v2.png",
       html_favicon_url = "http://www.rust-lang.org/favicon.ico",
       html_root_url = "http://doc.rust-lang.org/")]
```

This sets a few different options, with a logo, favicon, and a root URL.

## Generation options

`rustdoc` also contains a few other options on the command line, for further customization:

- `--html-in-header FILE`: includes the contents of FILE at the end of the
  `<head>...</head>` section.
- `--html-before-content FILE`: includes the contents of FILE directly after
  `<body>`, before the rendered content (including the search bar).
- `--html-after-content FILE`: includes the contents of FILE after all the rendered content.

## Security note

The Markdown in documentation comments is placed without processing into
the final webpage. Be careful with literal HTML:

```rust
/// <script>alert(document.cookie)</script>
# fn foo() {}
```
