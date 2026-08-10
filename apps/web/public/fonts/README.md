# Bundled typefaces

All three are SIL Open Font License 1.1, which permits bundling and redistribution inside a
commercial product provided the licence travels with them and the fonts are not sold on their own.

| Family | Used for | Upstream |
|---|---|---|
| Inter | interface text | https://github.com/rsms/inter |
| Be Vietnam Pro | transcript reading mode | https://github.com/lettersoup/Be-Vietnam-Pro |
| JetBrains Mono | timestamps, durations, counters | https://github.com/JetBrains/JetBrainsMono |

Only the `vietnamese`, `latin` and `latin-ext` subsets are shipped, at the weights the interface
actually uses. Regenerate with the snippet in `docs/fonts.md` if a new weight is needed.

Be Vietnam Pro exists in this list for a specific reason: it was drawn for Vietnamese, so stacked
diacritics (`ề`, `ỗ`, `ự`, `ườ`) keep their spacing at reading sizes instead of colliding with the
line above — which is what happens with most Latin faces once a tone mark sits on top of a
circumflex.
