# rinja

`rinja` is based on [Askama](https://crates.io/crates/askama).
It implements a template rendering engine based on [Jinja](https://jinja.palletsprojects.com/), and
generates type-safe Rust code from your templates at compile time
based on a user-defined `struct` to hold the template's context.
See below for an example.

All feedback welcome. Feel free to file bugs, requests for documentation and
any other feedback to the [issue tracker][issues].

You can find the documentation about our syntax, features, configuration in our book:
[rinja.readthedocs.io](https://rinja.readthedocs.io/).

### Feature highlights

* Construct templates using a familiar, easy-to-use syntax
* Benefit from the safety provided by Rust's type system
* Template code is compiled into your crate for optimal performance
* Optional built-in support for Actix, Axum, Rocket, and warp web frameworks
* Debugging features to assist you in template development
* Templates must be valid UTF-8 and produce UTF-8 when rendered
* Works on stable Rust

### Supported in templates

* Template inheritance
* Loops, if/else statements and include support
* Macro support
* Variables (no mutability allowed)
* Some built-in filters, and the ability to use your own
* Whitespace suppressing with '-' markers
* Opt-out HTML escaping
* Syntax customization

[issues]: https://github.com/rinja-rs/rinja/issues


How to get started
------------------

First, add the rinja dependency to your crate's `Cargo.toml`:

```sh
cargo add rinja
```

Now create a directory called `templates` in your crate root.
In it, create a file called `hello.html`, containing the following:

```jinja
Hello, {{ name }}!
```

In any Rust file inside your crate, add the following:

```rust
use rinja::Template; // bring trait in scope

#[derive(Template)] // this will generate the code...
#[template(path = "hello.html")] // using the template in this path, relative
                                 // to the `templates` dir in the crate root
struct HelloTemplate<'a> { // the name of the struct can be anything
    name: &'a str, // the field name should match the variable name
                   // in your template
}

fn main() {
    let hello = HelloTemplate { name: "world" }; // instantiate your struct
    println!("{}", hello.render().unwrap()); // then render it.
}
```

You should now be able to compile and run this code.
