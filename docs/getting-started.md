# Bắt đầu cùng làm Lim Harness

## Yêu cầu trên Windows

- PowerShell 5.1 hoặc PowerShell 7.
- Node.js **22.18 trở lên**, npm và Git. Ưu tiên Node LTS.
- Với desktop/build: Rust dùng toolchain **Windows MSVC**, Microsoft Visual Studio Build Tools với workload **Desktop development with C++**, Windows 10/11 SDK.
- Microsoft Edge WebView2 Evergreen Runtime để mở cửa sổ desktop.
- Ollama chỉ cần khi chạy inference, không cần để build hoặc xem frontend. Mỗi máy tự tải model và tự chọn vault Obsidian.

Nguồn cài đặt: [Node.js](https://nodejs.org/), [Git](https://git-scm.com/downloads/win), [Rust](https://rustup.rs/), [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows), [Ollama](https://ollama.com/download/windows). Sau khi cài công cụ, mở terminal mới để nhận PATH.

## Clone và khởi động

```powershell
git clone https://github.com/YOUR_USER/lim-harness.git
cd lim-harness
powershell -NoProfile -ExecutionPolicy Bypass -File .\start.ps1
```

Thay URL bằng repo thật. `ExecutionPolicy Bypass` chỉ áp dụng cho tiến trình chạy lệnh này, không đổi policy của máy.

Script kiểm tra môi trường, chạy `npm ci` theo lockfile khi cần, rồi mở Tauri dev. Lần đầu Rust sẽ tải/biên dịch dependency nên có thể mất thời gian. Không tự cài công cụ hệ thống, không tạo Git repo, không thay remote, không sửa Odysseus/vault, không tải hoặc nạp model.

### Các chế độ

```powershell
# Chỉ đọc môi trường; exit 1 nếu thiếu dependency bắt buộc
powershell -NoProfile -ExecutionPolicy Bypass -File .\check.ps1

# Frontend không cần Rust/C++/Ollama
powershell -NoProfile -ExecutionPolicy Bypass -File .\check.ps1 -Mode Web
powershell -NoProfile -ExecutionPolicy Bypass -File .\start.ps1 -Mode Web

# Chỉ cài dependency của repo, chưa mở app
powershell -NoProfile -ExecutionPolicy Bypass -File .\start.ps1 -Mode Setup

# Build executable debug, chưa tạo installer
powershell -NoProfile -ExecutionPolicy Bypass -File .\start.ps1 -Mode Build

# Cài lại node_modules theo lockfile
powershell -NoProfile -ExecutionPolicy Bypass -File .\start.ps1 -RefreshDependencies
```

`start.ps1` dùng thư mục chứa script, nên gọi được từ thư mục khác. Dấu hash trong `node_modules/.lim-lockhash` giúp cài lại khi `package-lock.json` thay đổi sau pull. Setup cũng fetch Rust dependency bằng `cargo fetch --locked`.

Web: http://127.0.0.1:1420/ chỉ là preview UI, không có native IPC/inference. Desktop: Ollama tại `127.0.0.1:11434`; chọn/tải model qua Cookbook. Chưa cần `.env` hay API key để dùng local Ollama.

Nếu cổng 1420 đang có server, script báo lỗi và yêu cầu dừng bằng Ctrl+C; không tự kill process. Nếu executable đang mở khiến build bị khóa, đóng bản app trước đó rồi build lại. `check.ps1` là kiểm tra dependency, không thay thế build hay kiểm tra model thật; WebView2 là phát hiện registry best effort.

## Đưa thư mục hiện tại lên Git lần đầu

Tạo một repo trống trên GitHub/GitLab trước. Các lệnh dưới dành cho thư mục Lim Harness chưa có root `.git`; không chạy trong `odysseus/`.

```powershell
git init -b main
git status --short --ignored
git add .
git diff --cached --stat
git diff --cached
# Xem nội dung staged trước khi commit, bỏ dữ liệu riêng nếu có.
git commit -m "Initial Lim Harness workspace"
git remote add origin https://github.com/YOUR_USER/lim-harness.git
git push -u origin main
```

Nếu đã có repo/remote, dùng `git status` và `git remote -v` để tiếp tục repo hiện có; không init hoặc thay remote một cách máy móc. Chưa có license công bố cho mã Lim Harness trong repo; nhóm cần chốt nếu định phát hành công khai.

### Những gì nên commit

`src/`, `public/`, `src-tauri/src/`, icons/capabilities/config/build.rs, `scripts/`, `check.ps1`, `start.ps1`, `package.json`, **package-lock.json**, **Cargo.lock**, tài liệu và design-system. Hai lockfile cần giữ trong Git để mọi máy dùng cùng bản dependency. Không sửa dependency bằng cách xóa lockfile.

`.gitignore` loại node_modules, dist, target, schema sinh tự động, artifact, model weights, database/chat, .env, cấu hình máy, vault và Notion pending. `.gitattributes` thống nhất newline. `.gitignore` không tự bỏ các tệp đã được track trước đó.

`odysseus/` là mẫu đọc riêng trên máy chủ dự án, đang bị ignore và không phải dependency của Lim. Bạn cùng nhóm không cần thư mục này để chạy app. Nếu cần chia sẻ mẫu, dùng một repo/thư mục riêng với nguồn và giấy phép AGPL-3.0 của Odysseus được giữ nguyên. `.agents/` chứa skill local và cũng không phải dependency chạy app.

## Cách cùng làm

- Mỗi người dùng branch riêng: `git switch -c feature/ten-cong-viec`.
- Pull trước khi làm; chia rõ phạm vi file để giảm conflict, mở PR để review trước khi merge.
- Không commit password, token, hội thoại cá nhân, `.env`, vault hoặc model weights.
- Đọc `AGENTS.md` và tài liệu trong `docs/`; tiến độ dự án nằm trên Notion. Quyền truy cập Notion cấp riêng cho từng thành viên; không có connector thì ghi pending theo quy tắc dự án.
- Khi đổi dependency, commit cả manifest và lockfile tương ứng. Khi gửi kết quả, nói rõ đã build/chạy kiểm tra gì và phần chưa xác nhận.

## macOS/Linux

PowerShell scripts này hỗ trợ Windows. Trên nền tảng khác, cài dependency hệ thống theo tài liệu Tauri rồi dùng `npm ci`, `npm run dev` hoặc `npm run tauri dev`. Chưa xác nhận build Lim trên macOS/Linux; hardware scan hiện ưu tiên Windows/NVIDIA.
