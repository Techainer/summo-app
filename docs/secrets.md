# Secrets này cần gì, và không có thì mất gì

Repo build được, test được và phát hành được **mà không cần secret nào**. Mỗi mục dưới đây mở thêm
một thứ; thiếu thì phần đó im lặng bỏ qua chứ không làm hỏng bản phát hành — và mỗi workflow đều in
ra một dòng nói rõ nó đã bỏ qua cái gì.

Đặt bằng `gh secret set`, đọc từ file hoặc từ stdin. **Đừng bao giờ để giá trị lên dòng lệnh** — nó
nằm lại trong history của shell và hiện trong `ps` của mọi tiến trình khác trên máy.

## Ký bản Android

Không có bốn secret này: `.apk` vẫn được build mỗi lần tag và vẫn nằm trong artifact của workflow,
chỉ không được đính vào release — vì một `.apk` không ký thì tải về xong điện thoại từ chối cài, và
người tải không có cách nào biết vì sao.

| Secret | Là gì |
| --- | --- |
| `ANDROID_KEYSTORE_BASE64` | keystore, mã hoá base64 |
| `ANDROID_KEYSTORE_PASSWORD` | mật khẩu của keystore |
| `ANDROID_KEY_ALIAS` | tên alias trong keystore (`summo`) |
| `ANDROID_KEY_PASSWORD` | mật khẩu của key; thường trùng mật khẩu keystore |

```bash
gh secret set ANDROID_KEYSTORE_BASE64   --repo Techainer/summo-app < summo.keystore.base64
gh secret set ANDROID_KEYSTORE_PASSWORD --repo Techainer/summo-app < keystore-password.txt
gh secret set ANDROID_KEY_PASSWORD      --repo Techainer/summo-app < keystore-password.txt
printf 'summo' | gh secret set ANDROID_KEY_ALIAS --repo Techainer/summo-app
```

Kiểm tra keystore trước khi đặt, để không phải đoán khi build đỏ:

```bash
keytool -list -v -keystore summo.keystore | grep -E "Alias name|Valid from"
base64 -d < summo.keystore.base64 | cmp - summo.keystore && echo "base64 khớp"
```

`release.yml` **không tin** vào việc secret có tồn tại: nó chạy `apksigner verify` trên đúng file vừa
build, và chỉ đính vào release nếu file thật sự đã ký. Có key mà build ra file chưa ký thì job đỏ,
kèm câu nói mắt xích nào đứt.

Token dùng để chạy `gh secret set` cần quyền **Secrets: write** trên repo. Fine-grained PAT mặc định
không có, và lỗi nó trả về là `HTTP 403: Resource not accessible by personal access token`.

## Ký và notarize bản macOS

Không có: bản `.dmg` được ký ad-hoc. Nó **chạy được** — chữ ký hợp lệ, `codesign` trả `valid on
disk` — nhưng Gatekeeper chưa có ai bảo lãnh, nên lần đầu mở người dùng phải vào Cài đặt hệ thống ›
Quyền riêng tư & Bảo mật › **Mở dù sao**, một lần. Cửa sổ `.dmg` đã in sẵn hai bước đó.

Có đủ sáu secret thì tag kế tiếp tự ký bằng Developer ID và tự notarize, người dùng không phải bấm
gì cả. Chi phí là 99 đô/năm cho tài khoản Apple Developer.

| Secret | Là gì |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 của file `.p12` (Developer ID Application) |
| `APPLE_CERTIFICATE_PASSWORD` | mật khẩu lúc export `.p12` |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Techainer (TEAMID)` |
| `APPLE_ID` | Apple ID dùng để notarize |
| `APPLE_PASSWORD` | app-specific password của Apple ID đó |
| `APPLE_TEAM_ID` | mã team, 10 ký tự |

Đặt `APPLE_CERTIFICATE` rỗng khác với không đặt: Tauri thấy có certificate, chạy `security import`
trên chuỗi rỗng, và build chết với `failed to import keychain certificate`. Đó là chuyện đã xảy ra
với v0.2.3.

## Dựng lại trang chủ khi có bản mới

| Secret | Là gì |
| --- | --- |
| `SITE_DEPLOY_HOOK` | URL deploy hook của Cloudflare cho `summo-site` |

Nút tải trên `summo.techainer.com` trỏ thẳng vào tệp — `Summo_0.2.8_aarch64.dmg`, kèm dung lượng —
và danh sách đó đọc từ GitHub **lúc dựng trang**. Trang chỉ dựng khi repo trang đổi, không phải khi
app phát hành, nên giữa hai lần đó nút tải mời bản cũ. Job `site` trong `release.yml` gọi hook này
sau khi bundle và installer xong.

Không đặt cũng không sao: trang hiện bản nó được dựng cùng và ghi rõ là bản nào. URL chính là thông
tin bí mật ở đây, nên workflow đưa nó cho `curl` qua stdin chứ không qua tham số.

Lấy URL ở Cloudflare › Workers & Pages › `summo-site` › Settings › Builds › Deploy hooks.

```bash
gh secret set SITE_DEPLOY_HOOK --repo Techainer/summo-app   # dán URL, Ctrl-D
```
