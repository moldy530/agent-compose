# invalid-encoding

## What it protects

Spec files are UTF-8 **without a byte-order mark**. A BOM is invisible in most
editors and is not whitespace to a YAML parser: it becomes part of the first
key, so `version:` stops being `version:` and every diagnostic after it is about
a document that does not look like the one on the screen.

Refusing the file outright, with one message naming the BOM, is cheaper for a
reader than the cascade.

## A spec that triggers it

Not expressible in a document: it is a property of the file's first bytes, not
of anything you can write. The file is one whose first three bytes are
`EF BB BF`, or which is not valid UTF-8 at all.

## The fix

Re-save the file as UTF-8 without a BOM. Most editors call the setting "UTF-8"
as opposed to "UTF-8 with BOM"; `sed -i '1s/^\xEF\xBB\xBF//' main.yml` strips
one in place.

Grammar: `docs/grammar.md` §1.1. Topic: `agent-compose docs getting-started`.
