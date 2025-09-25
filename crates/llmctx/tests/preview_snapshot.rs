use insta::assert_snapshot;

#[test]
fn preview_default_renders() {
    let rendered = "```
fn main() {
    println!(\"hello\");
}
```";
    assert_snapshot!("preview_default", rendered);
}
