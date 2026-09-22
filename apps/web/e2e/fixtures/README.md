# Fixtures

## `vi-fleurs.wav`

7.2 seconds of read Vietnamese, 16 kHz mono, from Google's **FLEURS** `vi_vn` test split, clip
`11098060233367360519`:

> Hiển nhiên, nếu bạn biết một ngôn ngữ La Mã, bạn sẽ dễ dàng học Tiếng Bồ Đào Nha.

FLEURS is published under **CC-BY-4.0** by Google, from the FLoRes-101 corpus. Redistribution is
allowed with attribution, which is what this file is.

### Why a real recording is committed rather than generated

`full-flow.mjs` asserts that speech becomes text on screen. A generated tone produces no
transcript, so the test would pass against a broken recogniser; synthesising speech at test time
would mean a text-to-speech model in CI, which is a larger download than this file and one more
thing to be flaky.

230 KB. It is a _test_ input, not a benchmark: the suite checks that lines arrive, never which
words they contain, because asserting the words would turn every model change into a broken test.

## `bilingual.wav`

22.4 seconds, the same format and the same corpus, four clips with six-tenths of a second of
silence between them so the detector closes each one:

| #   | Language | FLEURS clip            | Said                                                            |
| --- | -------- | ---------------------- | --------------------------------------------------------------- |
| 1   | `vi_vn`  | `11967436547438172966` | Nó mang đến cho chúng ta xe lửa, xe hơi và nhiều phương tiện…   |
| 2   | `en_us`  | `10233995782544396174` | Many people don't think about them as dinosaurs…                |
| 3   | `vi_vn`  | `10970996098314274277` | Tuy nhiên loài chim vẫn có rất nhiều điểm giống với khủng long. |
| 4   | `en_us`  | `10969774925516121312` | This is an important way to distinguish between some verbs…     |

### Why a second recording, at three times the size

`bilingual.mjs` pairs a Vietnamese-only model with a multilingual one and turns on two subtitle
languages. Every decision in that feature is about **which language a sentence was in** — which
model claims it, which subtitle it is owed, which one it must not be given — and none of them can
be observed on a recording with one language in it. The suite said so in its own header: provoking
an English sentence out of a Vietnamese fixture was not something it could arrange, so the routing
was left to unit tests and the two halves were never run together.

Two bugs were living in that gap when this file was added, and both are now asserted here.

Alternating rather than one language then the other: a meeting is a conversation, and a model that
only handles a language change at the halfway mark would pass a fixture built that way.
