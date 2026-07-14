# Hello, Fable

*A whirlwind tour of the language this site is built with.*

Fable is expression-oriented: `if` and `match` produce values, and the
last expression of a block is its result. Here is the classic first
program:

```fable
fn greet(name: String) -> String {
    "Hello, {name}!"
}

println(greet("world"));
```

## Sum types and matching

Pattern matching is exhaustive — forget a case and the compiler tells
you *before* the program runs:

```fable
enum Shape {
    Circle(Float),
    Rect(Float, Float),
}

fn area(s: Shape) -> Float {
    match s {
        Shape.Circle(r) -> math.pi * r * r,
        Shape.Rect(w, h) -> w * h,
    }
}
```

## Errors are values

Fallible operations return `Result`, and the `?` operator threads
failures for you. This is exactly how the page you are reading got
here — read, convert, write, with every error carried to one place:

```fable
fn build_page(src: String, dst: String) -> Result[Unit, String] {
    let text = fs.read(src)?;          // Err propagates automatically
    fs.write(dst, to_html(text))?;
    Ok(())
}
```

No exceptions, no `null` — just data. **The compiler keeps the
receipts.**
