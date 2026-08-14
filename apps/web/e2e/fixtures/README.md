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

230 KB, and it is the only binary in the repository. It is a *test* input, not a benchmark: the
suite checks that lines arrive, never which words they contain, because asserting the words would
turn every model change into a broken test.
