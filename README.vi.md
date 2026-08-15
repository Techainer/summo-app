[English](README.md) | **Tiếng Việt**

# Summo

[![CI](https://github.com/Techainer/summo-app/actions/workflows/ci.yml/badge.svg)](https://github.com/Techainer/summo-app/actions/workflows/ci.yml)
[![Licence: AGPL v3](https://img.shields.io/badge/licence-AGPL--3.0-blue.svg)](LICENSE)

Summo là ứng dụng ghi và ghi chép cuộc họp chạy ngay trên máy của bạn: ghi âm hoặc nhập một bản ghi
có sẵn, tạo transcript có gắn tên người nói, rồi lưu kết quả thành file Markdown mà bạn có thể mở,
grep, đồng bộ hay xoá tuỳ ý. Nhận dạng giọng nói và tách người nói không bao giờ rời khỏi máy; chỉ
có phần tóm tắt và dịch mới gọi ra một model ngôn ngữ, và chỉ khi bạn tự cấu hình nó.

Trang sản phẩm: [summo.techainer.com](https://summo.techainer.com).

## Ba nguyên tắc

Phần tốn kém nhất của một ứng dụng ghi họp là nhận dạng giọng nói theo thời gian thực. Chạy phần đó
ngay trên máy loại bỏ hoàn toàn chi phí GPU — đó là lý do có thể bán phần mở rộng trên cloud với giá
rẻ — và nghĩa là bản ghi cuộc họp của bạn là một file trên đĩa của bạn, chứ không phải một dòng trong
cơ sở dữ liệu của ai khác. Ba nguyên tắc sau đây theo từ đó, và chúng quyết định phần lớn thiết kế:

1. **Nhận dạng giọng nói và diarisation (tách người nói) luôn chạy local.** Không có đường lùi nào
   gọi ra ASR trên cloud.
2. **Bấm ghi là ghi ngay** — bắt đầu ghi trong chưa đầy một giây, không có hộp thoại nào chắn đường.
   Tóm tắt chỉ chạy sau khi cuộc họp kết thúc, vì đó mới là lúc bạn cần nó.
3. **Dữ liệu của bạn nằm ở `~/.summo/vault`** — các file Markdown bạn có thể mở bằng Obsidian, grep,
   backup hay xoá theo ý mình. Cùng một đường dẫn trên mọi hệ điều hành, gõ được từ trí nhớ.

## Hiện đã dùng được gì

Đã dùng được từ đầu đến cuối: ghi âm hoặc nhập một bản ghi có sẵn, có transcript gắn tên người nói,
một bản tóm tắt do agent soạn để bạn duyệt, việc cần làm trên bảng kanban, hỏi đáp trả lời từ kho dữ
liệu kèm trích dẫn, dịch trực tiếp nội dung đang phát, lồng tiếng (dubbing), lịch, bình luận, một
dàn agent bạn chỉnh sửa như file, và đồng bộ mã hoá giữa các máy qua bất kỳ thư mục dùng chung nào.
Ghi chú viết trong trình soạn thảo khối: có bảng, ảnh, kéo thả để đổi thứ tự, và trang lồng trong
trang — trên một file vẫn là Markdown, và mở ở dạng văn bản thuần thay vì làm mất thứ nó chưa giữ
được. Đăng ký lịch bằng URL thì lịch luôn cập nhật, sắp tới giờ họp app hỏi có ghi chú không —
không bao giờ tự ghi âm — và họp xong thì soạn sẵn thư gửi đi để bạn tự gửi.

**Chưa xong:** bản Android đã build, ghi âm được, tự ký khi kho mã có khoá, và đã chạy thật — trên
máy ảo, và chính ở đó mới lòi ra rằng bản release không kết nối nổi tới engine của chính nó. Phần
nhận dạng đã được biên dịch vào bản cho điện thoại nhưng chưa từng giải mã âm thanh trên máy thật:
ONNX Runtime không phát hành bản x86-64 cho Android, mà máy ảo thì không chạy được bản arm64. Cứ coi
như điện thoại vẫn chưa kiểm chứng. iOS cần máy Mac nên chưa từng build. Relay đồng bộ trên cloud (hosted sync relay) cũng chưa
được xây — hiện tại đồng bộ vẫn chạy qua một thư mục dùng chung.

Các con số dưới đây đều đo trên chính codebase này, không phải ước lượng. Mỗi con số tái lập được
bằng đúng lệnh ghi kèm; xem đầy đủ phương pháp và các lưu ý ở
[`docs/benchmarks.md`](docs/benchmarks.md) và [`docs/translation.md`](docs/translation.md).

| Nhận định                           | Đo được                                                                                                                                | Nguồn                                                                                                 |
| ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Độ chính xác nhận dạng tiếng Việt   | 8,5 % WER, 6,7 % CER (`gipformer-65M`, 100 clip FLEURS VI, 21,3 phút; còn 5,3 % nếu bỏ các clip mà bản tham chiếu viết số bằng chữ số) | `cargo run --release -p summo-bench --features asr -- asr`                                            |
| Tốc độ pipeline chạy live           | RTF 0,107, tức nhanh hơn thời gian thực khoảng 9 lần (ghi bằng mic thô)                                                                | `docs/benchmarks.md`, mục pipeline đầu-cuối — mới đo trên hai đoạn ghi ngắn từ một mic, chưa tính WER |
| Voice activity detection (VAD)      | Silero v5, F1 0,940 (precision 0,925, recall 0,956)                                                                                    | `cargo run --release -p summo-bench --features silero -- vad --sweep`                                 |
| Tìm một cuộc họp mà không cần index | ~30 ms trên 1.000 cuộc họp (scan 8 luồng) — đây cũng là lý do không có database                                                        | `cargo run --release -p summo-bench -- vault --sizes 100,1000,5000`                                   |
| Dịch một dòng                       | ~244 ms/dòng, 8 luồng, với model mặc định `small100` nặng 583 MB — chạy ngay trong binary phát hành, không cần dựng model server       | `cargo run -p summo-mt --features local,onnx --example compare`                                       |

## Cài đặt và chạy

Một lệnh duy nhất, theo đúng cách `ollama` là một lệnh. Giao diện được biên dịch thẳng vào binary,
nên không có web server nào cần khởi động, cũng không có thư mục static file nào phải giữ cho khớp.

```bash
./scripts/bundle.sh          # tạo tarball trong dist/, sẵn sàng chép sang máy khác
tar -xzf dist/summo-*.tar.gz && cd summo-* && ./summo serve
```

Mỗi bản phát hành có bốn bản build: Linux x64 và arm64, macOS trên chip Apple, và Windows x64. Không
có macOS Intel, vì ONNX Runtime không còn phát hành bản build cho nền tảng đó — bản 1.28 và 1.29 của
Microsoft chỉ có `osx-arm64`. Build từ source trên máy Mac Intel cũng dừng ở đúng chỗ này, chỉ là
báo lỗi rõ ràng hơn.

### Bản app, thay vì dòng lệnh

```bash
pnpm -C apps/desktop exec tauri build     # .deb và .AppImage, .dmg, .msi
```

Một cửa sổ, một icon ở khay hệ thống, và `⌘⇧R` gọi được từ bất cứ đâu. Nó khởi động đúng cái daemon
mà lệnh ở trên chạy — và nếu đã có một cái đang chạy thì dùng luôn thay vì tranh nhau — nên hai
đường là cùng một sản phẩm, cùng một vault, dùng đường nào cũng được.

Bản phát hành có kèm installer bên cạnh tarball. **Chúng chưa được ký số**: macOS sẽ báo không xác
minh được nhà phát triển, Windows sẽ hiện cảnh báo SmartScreen — vì không có chứng chỉ Apple, cũng
không có chứng chỉ Windows. Tarball không vướng chuyện này, và là thứ nên dùng nếu bạn ngại điều đó.

Hoặc build từ source, không đóng gói:

```bash
pnpm install && pnpm --filter @summo/web build       # build giao diện, một lần
cargo run -p summo-cli --features serve,models -- serve
```

Lệnh trên in ra một địa chỉ và tự mở nó. Lần chạy đầu tiên chỉ có đúng một quyết định phải đưa ra:
tải model nhận dạng nào. Summo xếp hạng những model phù hợp với máy của bạn và giải thích vì sao —
dung lượng RAM, real-time factor đã đo, giấy phép — và bạn có thể không đồng ý với gợi ý đó. Không có
gì được ghi lại cho tới khi bạn bấm nút ghi.

```bash
summo serve --port 8710      # cố định một port, khi có thứ khác cần tìm tới nó
summo serve --background     # chạy nền; `summo status` để xem, `summo stop` để dừng
summo serve --no-open        # chạy server, khi không có trình duyệt nào để mở
summo import ~/Downloads/zoom-recording.mp4
summo mcp                    # đưa kho dữ liệu ra qua MCP, cho Claude Code hay Cursor
```

Có hai cách build bản release: kèm nhận dạng giọng nói (thư viện ONNX Runtime và sherpa-onnx nằm
cạnh binary), hoặc với `--no-models` — nhỏ hơn, vẫn duyệt được kho dữ liệu, nhập file, tóm tắt và trả
lời câu hỏi, nhưng không transcribe được.

## Danh mục model

Summo không đóng gói sẵn model nào. Mọi model được tải về lúc chạy từ một registry gồm các manifest
JSON tĩnh, mỗi manifest ghi rõ giấy phép, sha256 của từng file, và các con số đã đo được:

```bash
summo recommend --lang vi     # máy này chạy được gì, và vì sao
summo pull gipformer-65m      # 2,4 % WER trên Fleurs VI, ~70 MB, MIT
```

Danh mục nằm ở [Techainer/summo-registry](https://github.com/Techainer/summo-registry) (giấy phép
MIT, toàn JSON tĩnh, fork và mirror thoải mái). Một model được phân giải theo chuỗi
`SUMMO_REGISTRY` → CDN của chúng tôi → repo registry trên GitHub → URL bên trong manifest, trỏ tới
đúng nơi phát hành bộ trọng số đó — model có giấy phép permissive thì được mirror lại, còn model
`gated` hay non-commercial thì trỏ thẳng về nơi gốc (thường là repo trên Hugging Face được ghi trong
`files[].url` của từng manifest), nên Summo không bao giờ là bên phân phối một giấy phép mà mình
không có quyền phân phối lại.

## Kiểm chứng lời hứa về quyền riêng tư

Ngắt kết nối mạng của máy, rồi chạy `./summo serve` và ghi một cuộc họp. Nhận dạng giọng nói, VAD và
tách người nói vẫn chạy bình thường, vì chúng chưa bao giờ gọi ra ngoài — không có đường lùi nào sang
ASR trên cloud để mà thất bại. Điều bạn _sẽ không_ làm được khi offline là lấy một bản tóm tắt hay
bản dịch từ model từ xa mà bạn đã cấu hình — đó là ngoại lệ duy nhất, có chủ đích, của lời hứa "không
gì rời khỏi máy".

Phần còn lại của lời hứa này được thực thi bằng code, không chỉ nằm ở mô tả — xem
[`SECURITY.md`](SECURITY.md): daemon chỉ bind vào `127.0.0.1`, mọi route đều cần token ghi trong
`~/.summo/engine.json`, và một trang web thường không thể chạm tới daemon trừ khi nó được khởi động
với `--dev` — điều không bản build phát hành nào làm.

## Kiến trúc

```
crates/
  summo-core      kiểu dữ liệu dùng chung: segment, event, path, error
  summo-models    registry model kiểu Ollama: manifest, tải resumable, blob store, dò phần cứng
  summo-vad       voice activity detection: nhiều backend, và cổng chia segment nuôi ASR
  summo-asr       phiên decode: pseudo-streaming, hybrid refine, runtime sherpa
  summo-diar      tách người nói: track prior, online clustering, tinh chỉnh
  summo-vault     kho Markdown: mỗi cuộc họp là một file bạn sở hữu
  summo-llm       tóm tắt, dịch và hỏi đáp — phần duy nhất rời khỏi máy
  summo-engine    daemon local: capture, nhận dạng và event qua loopback
  summo-cli       `summo serve | setup | pull | import | ask | export | registry`
  summo-agent     agent: lõi aionrs, cùng bộ tool riêng của Summo
  summo-mcp       đưa kho dữ liệu ra qua MCP — tool, resource, prompt, qua stdio hoặc HTTP
  summo-sync      đồng bộ đa máy dùng CRDT, mã hoá đầu-cuối
apps/
  web/            giao diện ứng dụng — React, biên dịch thẳng vào binary
  desktop/        vỏ Tauri: cửa sổ, tray, phím tắt toàn cục
  mobile/         Tauri iOS/Android — Android đã ra .apk; iOS chưa từng build
```

`summo-bench`, `summo-audio`, `summo-media`, `summo-calendar`, `summo-tts`, `summo-store`,
`summo-mt` và `summo-pipeline` lần lượt lo phần đo đạc, capture, ffmpeg, lịch, lồng tiếng, tìm kiếm
ngữ nghĩa, dịch trong tiến trình và các chặng của pipeline — xem thư mục `crates/` để biết đủ hai
mươi crate.

## Đóng góp

- [CONTRIBUTING.md](CONTRIBUTING.md) — cách chạy dự án, và mọi kiểm tra CI chạy, để bạn tự chạy trước
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [SECURITY.md](SECURITY.md) — sản phẩm cam kết những gì, và báo lỗ hổng ở đâu
- [docs/adr/](docs/adr/) — những quyết định định hình dự án, và điều gì sẽ mở lại từng quyết định đó

## Giấy phép

AGPL-3.0-or-later. Model được tải về lúc chạy và giữ nguyên giấy phép riêng của chúng; xem
[`NOTICE`](NOTICE). Bản thân ứng dụng được tách khỏi phần mở rộng chạy trên cloud một cách có chủ
đích: repo này không import bất cứ thứ gì từ `summo-cloud` — repo độc quyền (proprietary) vận hành
CDN, billing và relay đồng bộ. Xoá `summo-cloud` đi thì Summo vẫn ghi âm, transcribe, cài model và
export bình thường, chỉ có đồng bộ là ngừng hoạt động.
