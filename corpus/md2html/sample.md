# Markdown to HTML

This is a **Markdown to HTML** converter written entirely in [Blood](https://blood-lang.org).

It handles *italic*, **bold**, and `code spans` inline.

## Features

- Headings from h1 through h6
- **Bold** and *italic* formatting
- `Inline code` with HTML escaping: `<div class="test">`
- [Hyperlinks](https://example.com) to external pages
- Fenced code blocks
- Unordered and ordered lists
- Blockquotes
- Horizontal rules

### Code Blocks

Here is a Blood function:

```
fn fibonacci(n: i32) -> i32 {
    if n <= 1 {
        return n;
    }
    fibonacci(n - 1) + fibonacci(n - 2)
}
```

## Ordered Lists

1. First item with **bold**
2. Second item with *italic*
3. Third item with `code`
4. Fourth item with a [link](https://example.com)

## Blockquotes

> This is a blockquote.
> It can contain **formatted** text and `code`.

---

## Special Characters

Ampersands & angle brackets < > are escaped properly.

Here is a paragraph that spans
multiple lines and should be
joined into a single paragraph.

---

###### The smallest heading

That's all!
