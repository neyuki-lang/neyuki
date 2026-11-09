# @neyuki/html

Parsing, querying and writing HTML. Parsing follows the WHATWG algorithm browsers use (through html5ever), so missing `<html>`, `<head>` and `<body>` tags are added, unclosed and misnested tags are repaired, and entities are decoded the way a browser would.

```lua
local html = require("@neyuki/html")
local http = require("@neyuki/http")

local page = html.parse(http.get("https://example.com").body)
for _, link in html.select(page, "a[href]") do
    print(html.text(link), link.attrs.href)
end
```

Nodes are plain tables, which a script can read, change or build from scratch:

| Node | Fields |
| --- | --- |
| document | `type = "document"` (or `"fragment"`), `children` |
| element | `type = "element"`, `tag` (lower case), `attrs` (name to value), `children` |
| text | `type = "text"`, `text` |
| comment | `type = "comment"`, `text` |
| doctype | `type = "doctype"`, `name` |

`children` is an array of nodes in document order. When building a tree, a plain string in `children` is a text node, and `type` may be left out of a table that has a `tag` or a `text`.

## html.parse(text: string): table

Parses a whole HTML document and returns its document node.

## html.parseFragment(text: string): table

Parses a piece of HTML as if it sat inside `<body>`, and returns a `"fragment"` node holding just its nodes, without a document around them.

```lua
local fragment = html.parseFragment("<li>one<li>two")
print(#fragment.children) --> 2
```

## html.select(node: table, selector: string): table

The elements under `node` (not `node` itself) that match a CSS selector, in document order. Supported: type, `*`, `#id` and `.class` selectors; attribute selectors `[a]`, `[a=v]`, `[a~=v]`, `[a|=v]`, `[a^=v]`, `[a$=v]` and `[a*=v]`, with an `i` flag for case-insensitive values; the descendant, `>`, `+` and `~` combinators; selector lists separated by commas; and the pseudo-classes `:first-child`, `:last-child`, `:only-child`, `:nth-child(an+b)`, `:nth-last-child()`, `:first-of-type`, `:last-of-type`, `:only-of-type`, `:nth-of-type()`, `:nth-last-of-type()`, `:empty`, `:root` and `:not()` (of simple selectors). Tag and attribute names match in any case. Raises an error for a malformed selector or an unsupported pseudo-class.

```lua
for _, cell in html.select(page, "table.scores tr:nth-child(odd) > td:first-child") do
    print(html.text(cell))
end
```

## html.selectOne(node: table, selector: string): table?

The first element under `node` that matches a CSS selector, or nil.

## html.text(node: table): string

The text of a node and everything under it, as the DOM's `textContent` gives it: text nodes joined, comments left out. Whitespace is kept as written.

## html.render(node: table): string

Writes a node and everything under it as HTML. Attributes are written sorted by name; `true` writes a bare attribute and `false` leaves it out. Text is escaped except inside `<script>`, `<style>` and the other raw-text elements, and void elements such as `<br>` and `<img>` get no end tag. Raises an error for a table that is not a node, a tag or attribute name that would break the markup, and output over 64 MiB.

```lua
print(html.render({ tag = "a", attrs = { href = "/" }, children = { "Home & away" } }))
--> <a href="/">Home &amp; away</a>
```

## html.escape(s: string): string

Replaces `&`, `<`, `>`, `"` and `'` with character references, so `s` can be placed in HTML text or a quoted attribute value.

```lua
print(html.escape('<b class="x">')) --> &lt;b class=&quot;x&quot;&gt;
```
