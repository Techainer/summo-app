[English](README.md) | **Tiếng Việt**

# Summo

[![CI](https://github.com/Techainer/summo-app/actions/workflows/ci.yml/badge.svg)](https://github.com/Techainer/summo-app/actions/workflows/ci.yml)
[![Bản mới nhất](https://img.shields.io/github/v/release/Techainer/summo-app?label=t%E1%BA%A3i%20v%E1%BB%81&color=2f9e6f)](https://github.com/Techainer/summo-app/releases/latest)
[![Giấy phép: AGPL v3](https://img.shields.io/badge/licence-AGPL--3.0-blue.svg)](LICENSE)

Summo là ứng dụng ghi và ghi chép cuộc họp chạy ngay trên máy của bạn: ghi âm hoặc nhập một bản ghi
có sẵn, tạo transcript có gắn tên người nói, rồi lưu kết quả thành file Markdown mà bạn có thể mở,
grep, đồng bộ hay xoá tuỳ ý. Nhận dạng giọng nói và tách người nói không bao giờ rời khỏi máy; chỉ
có phần tóm tắt và dịch mới gọi ra một model ngôn ngữ, và chỉ khi bạn tự cấu hình nó.

Trang sản phẩm: [summo.techainer.com](https://summo.techainer.com).

![Một buổi họp đang được ghi: thanh đỏ có đồng hồ và cột đo âm lượng, ghi chú bên trái, transcript chạy bên phải](docs/media/recording.webp)

<sub>Một buổi họp đang chạy. Thanh trên cùng là phần ghi âm — thời gian đã ghi, và một cột đo động
theo tiếng nói trong phòng, vì "đang ghi" và "có nghe thấy bạn" là hai chuyện hỏng riêng. Ghi chú
bên trái, bản ghi bên phải: cùng một tài liệu, ghi vào cùng một file Markdown.</sub>

| | |
| --- | --- |
| ![Trang chính: nút ghi kèm sóng âm, việc đang chờ, và các buổi gần đây](docs/media/home.webp) | ![Buổi họp đã xong: trình phát, xuất file, transcript bên cạnh bản tóm tắt](docs/media/meeting.webp) |
| **Trang chính.** Ghi, nhập file hoặc viết — bên cạnh là những việc đang chờ bạn. | **Sau buổi họp.** Bản ghi âm, transcript, và bản tóm tắt do agent soạn để bạn duyệt. |
| ![Danh mục mô hình: cái gì đang chạy trên máy này, rồi tới mọi model trong kho](docs/media/models.webp) | ![Trang chi tiết model: nguồn gốc, chi phí, và các số đo](docs/media/model-detail.webp) |
| **Mô hình.** Cái sẽ chạy khi bạn bấm ghi — nhận dạng, dò giọng, nhận diện người nói, dịch — nằm trên đầu, rồi mới tới cả kho. | **Mỗi model có trang riêng**: giấy phép, ai phát hành, bộ nhớ, tốc độ và độ chính xác đã đo, kèm checksum. |

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

Mỗi bản phát hành có bốn bản build: Linux x64 và arm64, macOS trên chip Apple, và Windows x64.

**Không có bản macOS Intel.** Trên giấy tờ thì có, suốt hai bản phát hành mà thực tế không hề có
file nào: GitHub đã bỏ runner `macos-13`, và một job xin đúng cái nhãn không còn máy nào chạy thì
nằm xếp hàng chứ không báo lỗi — nên nó trông như đang build mãi mãi, và không ai để ý là thiếu
file. Tự build vẫn được nếu bạn có máy Intel: `bundle.sh` vẫn xử lý `x86_64-apple-darwin`, và
`scripts/onnxruntime-intel-mac.sh` tải ONNX Runtime mà Microsoft ngừng phát hành sau 1.23.2 —
Summo nạp lúc khởi động thay vì link sẵn.

### Bản app, thay vì dòng lệnh

```bash
pnpm -C apps/desktop exec tauri build     # .deb và .AppImage, .dmg, .msi
```

Một cửa sổ, một icon ở khay hệ thống, và `⌘⇧R` gọi được từ bất cứ đâu. Nó khởi động đúng cái daemon
mà lệnh ở trên chạy — và nếu đã có một cái đang chạy thì dùng luôn thay vì tranh nhau — nên hai
đường là cùng một sản phẩm, cùng một vault, dùng đường nào cũng được.

Bản phát hành có kèm installer bên cạnh tarball. **Chúng chưa mua chứng chỉ Apple**, và đây là điều
đã được *đo* chứ không phải phỏng đoán — `.github/workflows/gatekeeper.yml` tải chính bản `.dmg` đã
phát hành, đánh dấu nó đúng như trình duyệt đánh dấu file tải về, rồi hỏi macOS:

```
Signature=adhoc
/Applications/Summo.app: rejected            (spctl)
/Applications/Summo.app: valid on disk       (codesign --verify --deep --strict)
/Applications/Summo.app: satisfies its Designated Requirement
Adhoc Signed App — Severity: Warning         (syspolicy_check, macOS 26.5)
Notary Ticket Missing
```

Bốn dòng đó nói hai chuyện khác nhau, và trước đây README này chỉ đọc dòng đầu. `rejected` nghĩa là
**không tự mở**, không phải **không mở được**: chữ ký vẫn hợp lệ và đúng Designated Requirement, nên
macOS hiện hộp thoại "không xác minh được nhà phát triển" và để lại nút **Mở dù sao** trong Cài đặt
hệ thống › Quyền riêng tư & Bảo mật. Bấm một lần, xong vĩnh viễn.

Chỉ khi chữ ký *hỏng* mới ra "Summo is damaged and can't be opened" — lúc đó không có nút nào cả và
Terminal là đường duy nhất. Bản v0.2.0 đúng là như vậy, vì nó hoàn toàn không được ký; câu chữ dặn
người dùng mở Terminal ra đời từ đó và ở lại lâu hơn nguyên nhân của nó.

Nếu bạn thích gõ hơn là bấm, dòng này làm cùng việc:

```bash
xattr -dr com.apple.quarantine /Applications/Summo.app
```

**Không có cách miễn phí nào bỏ hẳn bước đó.** Đường chính thức của Apple là Developer ID + notarize,
99 đô một năm. Phần việc phía Summo đã làm xong và đang chờ: đặt bốn secret (`APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID`)
là bản tag kế tiếp tự ký và tự notarize — xem `.github/workflows/release.yml`.

Windows hiện cảnh báo SmartScreen và vẫn cho bạn đi tiếp. Tarball không gặp cả hai vấn đề đó, và là
thứ nên dùng nếu bạn quan tâm.

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
- [docs/secrets.md](docs/secrets.md) — repo cần secret nào, và không có thì mất gì

## Giấy phép

AGPL-3.0-or-later. Model được tải về lúc chạy và giữ nguyên giấy phép riêng của chúng; xem
[`NOTICE`](NOTICE). Bản thân ứng dụng được tách khỏi phần mở rộng chạy trên cloud một cách có chủ
đích: repo này không import bất cứ thứ gì từ `summo-cloud` — repo độc quyền (proprietary) vận hành
CDN, billing và relay đồng bộ. Xoá `summo-cloud` đi thì Summo vẫn ghi âm, transcribe, cài model và
export bình thường, chỉ có đồng bộ là ngừng hoạt động.
