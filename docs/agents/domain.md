# Domain Docs

How engineering skills consume this repository's domain documentation.

## Before Exploring

Read the root `CONTEXT.md`, if it exists, and any relevant records under
`docs/adr/`. Their absence is not an error; proceed without creating them.

This is a single-context repository. The domain layout is:

```text
/
|- CONTEXT.md
|- docs/adr/
`- src/
```

`CONTEXT.md` is a glossary, not an implementation specification. Domain-modeling
creates it lazily when terminology is resolved. ADRs are also created lazily and
only for decisions that are hard to reverse, surprising without context, and
the result of a real tradeoff.

## Use The Glossary's Vocabulary

Use terms as defined in `CONTEXT.md` in issues, proposals, hypotheses, and test
names. If a required concept is absent, reconsider whether new terminology is
necessary or note the gap for domain-modeling.

## Flag ADR Conflicts

Surface conflicts with an existing ADR explicitly instead of silently
overriding the recorded decision.
