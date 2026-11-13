# Lim Harness

AI harness desktop xây bằng Tauri v2, React, TypeScript và Rust. Odysseus được giữ nguyên trong `odysseus/` làm tài liệu tham khảo; implementation mới không sao chép mã từ dự án đó.

## Khởi động nhanh trên Windows

Sau khi clone, mở PowerShell trong thư mục dự án:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\start.ps1
```

Script kiểm tra môi trường, cài dependency từ lockfile khi cần, rồi chạy desktop. Dùng `-Mode Web` để chỉ xem frontend, `-Mode Setup` để chuẩn bị dependency, `-Mode Build` để build debug executable. Chỉ kiểm tra môi trường: `powershell -NoProfile -ExecutionPolicy Bypass -File .\check.ps1`.

Chi tiết công cụ cần cài, Git push lần đầu và cách cùng làm: [Hướng dẫn setup](docs/getting-started.md). Ollama/model/vault được cấu hình riêng trên mỗi máy; không cần Odysseus để chạy Lim.

Muốn Agent tra cứu web ổn định, cài thêm SearXNG cục bộ (một lần mỗi máy): xem [Agent mode và tìm web (SearXNG)](#agent-mode-và-tìm-web-searxng).

## Chạy frontend

```sh
npm ci
npm run dev
```

Mở http://localhost:1420. `npm run build` kiểm tra TypeScript và build frontend. Node 22.18+ hoặc 24 chạy `npm run test:state` để kiểm tra đọc dữ liệu lưu, ngày không hợp lệ và lỗi localStorage.

`npm run test:colors` kiểm tra contrast chữ của các token sáng/tối; `npm run test:context` kiểm tra cách tính context; `npm run test:thinking` kiểm tra bộ tách các bước suy nghĩ. Font Geist được đóng gói kèm app, không tải font từ mạng.

## Chạy cửa sổ Tauri

Máy cần Rust/Cargo, Microsoft C++ Build Tools và WebView2 theo [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows).

```sh
npm run tauri dev
```

`npm run tauri build -- --no-bundle` build executable. Cấu hình hiện chưa tạo installer.

## Lõi Ollama và bộ nhớ

Mở Ollama tại `127.0.0.1:11434`, tải một model, rồi chạy `npm run tauri dev`. Trong Cài đặt, tải lại danh sách và chọn model đã cài. Browser tại localhost:1420 chỉ xem frontend, không có native IPC để chạy lõi.

- Hội thoại thật streaming qua Ollama; nút dừng hủy request, giữ phần đã nhận. Có timeout, giới hạn context và xử lý lỗi.
- SQLite `harness.sqlite3` trong thư mục app data `dev.lim.harness` lưu phiên, tin nhắn và trạng thái lượt. Lượt chưa kết thúc khi đóng app được khôi phục thành interrupted.
- Dữ liệu phiên cũ trong WebView được chuyển một lần khi kho native rỗng; assistant từ bản UI thử được gắn nguồn riêng và không gửi model. Browser preview và WebView có localStorage riêng.
- Chọn vault trong Cài đặt. Bật **Bộ nhớ cho vault** để tự ghi memory vào `LimMemory/` sau hội thoại; có thêm/sửa/ghim/quên, nguồn/lịch sử và tùy chọn dùng/tự ghi nhớ độc lập. Ghi chú ngoài thư mục này vẫn chỉ đọc. Chi tiết: [Memory v1](docs/memory-v1.md).
- Context mặc định 4K, chọn được 4K–128K theo mức tối đa model hỗ trợ (đọc từ Ollama). Ngân sách tính theo token ước lượng (~3 byte/token). Khi hội thoại vượt 75% chỗ trống, model tự tóm tắt các lượt cũ (auto-compact); có nút "Thu gọn ngay". Transcript gốc vẫn được giữ.
- Một giới hạn output dùng chung cho suy nghĩ và trả lời; hết giới hạn thì tin dừng ở trạng thái "đã dừng" với nút **Tiếp tục** (model Qwen suy nghĩ tiếp đúng mạch cũ).
- Cookbook có tab Active/Launch/Download/Dependencies/Settings và bảng model compact. Đọc cấu hình Windows, gợi ý theo RAM/VRAM, tìm/lọc/sắp xếp catalog và model đã cài. Download tải tag Ollama và báo tiến độ từng layer; Launch nạp/xác nhận trước khi chọn cho chat. Có ngắt yêu cầu và giải phóng model. [Hướng dẫn Cookbook](docs/cookbook.md).
- Catalog kèm app có 67 tag từ 20 family, metadata registry đã xác minh và cache/cập nhật online. Bảng đánh giá Auto/RAM/GPU, FIT SCORE phù hợp bộ nhớ và SPEED quan sát từ lượt chat khi có dữ liệu. [Nguồn và công thức](docs/model-catalog.md). `npm run catalog:refresh` thu thập snapshot metadata mới, không tải weights.

## Giao diện hiện tại

- Hội thoại, tạo/chuyển phiên, tìm phiên và ghi chú, đính kèm tệp .txt/.md, sao chép tin nhắn.
- Tạo/sửa ghi chú cục bộ; chọn module mẫu; cấu hình Ollama/context/vault và sáng/tối.
- Bố cục lấy cảm hứng từ Odysseus: sidebar thu gọn, lịch sử phiên ngay bên trái, chat rộng và composer gọn. Điều hướng gồm Hội thoại, Bộ nhớ, Cookbook, Module và Cài đặt.
- Cookbook, Bộ nhớ, Module và Cài đặt mở thành cửa sổ nổi trong app: kéo, đổi kích thước, thu gọn về dock và phóng to/khôi phục. Chat là vùng chính; chọn Hội thoại thu gọn các cửa sổ đang mở. Đóng cửa sổ không hủy công việc nền.
- Logo hình học riêng dùng trong UI, favicon và icon Tauri; vector gốc tại `public/logo.svg` và `public/app-icon.svg`.
- Ghi chú cục bộ và tùy chọn giao diện dùng localStorage; hội thoại native có SQLite làm nguồn chính.
- Agent mode (nút **Agent** ở ô soạn tin): model dùng công cụ tra web, đọc trang, xem thư mục, và — khi bạn bấm **Cho phép** — đọc/ghi tệp, chạy lệnh terminal. Module vẫn là danh mục UI mẫu, chưa được chạy bởi lõi hội thoại.

Đã kiểm tra với `qwen3.5:2b` qua Ollama trên Windows. Model nhỏ có thể trả lời sai hoặc bỏ qua chỉ dẫn; app luôn hiện các bước công cụ và cảnh báo khi câu trả lời chưa có nguồn web.

Memory v1 đã có fixture native với Ollama thật và vault tổng hợp để kiểm tra hàng đợi, ghi Markdown, recall giữa hai phiên và nguồn được lưu. Chưa xác nhận vault cá nhân hoặc CRUD bằng chuột trên desktop; chất lượng extraction/citation phụ thuộc model. Xem giới hạn trong [Memory v1](docs/memory-v1.md).

Thiết kế: `design-system/lim-harness/MASTER.md`. Quy tắc agent: `AGENTS.md`. Tiến độ: [Notion](https://app.notion.com/p/3f7cf6346c8581e0be15fda0adceca12).

## Tham số chạy model

Chat có panel inference + system prompt bên phải, ẩn/mở từ thanh tiêu đề. Mọi thông số (context, temperature, top P/K, repeat penalty, seed, system prompt, giới hạn output, tự thu gọn, Agent) lưu **riêng cho từng phiên** trong SQLite khi nhấn **Áp dụng**; phiên mới bắt đầu từ mặc định. Lượt đang chạy giữ cấu hình lúc gửi. Xem `docs/inference-panel.md` cho phạm vi tham số và ngân sách context.

## Agent mode và tìm web (SearXNG)

Bật **Agent** ở ô soạn tin. Mỗi lượt Agent bắt đầu bằng xem ngày giờ trên máy; câu hỏi về "hiện tại / mới nhất / hôm nay…" được tìm web kèm năm hiện tại trước khi trả lời.

| Công cụ | Việc làm | Cần bạn cho phép |
|---|---|---|
| `current_time` | Ngày giờ trên máy | Không |
| `web_search` | Tìm web (SearXNG → DuckDuckGo → Bing) | Không |
| `fetch_url` | Đọc nội dung trang http/https | Không |
| `list_dir` | Liệt kê thư mục | Không |
| `read_file` / `write_file` | Đọc / ghi tệp văn bản | **Có** |
| `run_command` | Chạy lệnh PowerShell (tối đa 60 giây) | **Có** |

### Cài SearXNG (khuyến nghị, một lần mỗi máy)

DuckDuckGo/Bing hay chặn truy vấn tự động, nên Agent ưu tiên một SearXNG chạy trên máy (gộp kết quả Google, Brave…; miễn phí, không cần API key). Không cần Docker hay WSL. Cần Git và **Python 3.10+ cài từ python.org** (không dùng bản Microsoft Store vì nó chuyển hướng venv vào AppData).

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\setup-searxng.ps1
```

Script tải mã nguồn chính thức `searxng/searxng`, tạo venv và cài thư viện trong `.local/` (đã gitignore):

- `.local/searxng-src/` — mã nguồn SearXNG (chỉ phần cần để chạy).
- `.local/searxng-venv/` — môi trường Python riêng.
- `.local/searxng-settings.yml` — chỉ nghe `127.0.0.1:8888`, bật JSON, tắt limiter, khóa bí mật ngẫu nhiên (không commit).

Chạy lại script để cập nhật SearXNG; file settings đã có được giữ nguyên. Thêm `-Start` để bật ngay sau khi cài (bản bật tay này không bị app tắt).

### Vòng đời

- **Mở app → SearXNG tự bật** (nếu đã cài). Nếu cổng 8888 đã có SearXNG khác, app chỉ dùng, không quản lý.
- **Đóng app → SearXNG tự tắt**, kể cả khi app bị dừng đột ngột: các tiến trình nằm trong Job Object của Windows và kết thúc cùng app.
- Chưa cài SearXNG: Agent vẫn tìm qua DuckDuckGo/Bing; khi không có nguồn, câu trả lời có cảnh báo "Chưa lấy được nguồn web nào…".
- Dùng SearXNG ở địa chỉ khác: đặt biến môi trường `LIM_SEARXNG_URL` (ví dụ `http://192.168.1.10:8080`) trước khi mở app; instance đó cần bật `formats: [html, json]`.

### Kiểm tra SearXNG có chạy

Trong Windows PowerShell, `curl` là tên gọi tắt của `Invoke-WebRequest`, nên dùng một trong hai lệnh (kết quả `200` = đang chạy):

```powershell
curl.exe -s -o NUL -w "%{http_code}\n" http://127.0.0.1:8888/healthz
```

```powershell
try { (Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8888/healthz -TimeoutSec 3).StatusCode } catch { "SearXNG không chạy" }
```

Thử tìm kiếm qua API JSON mà app dùng:

```powershell
(Invoke-RestMethod "http://127.0.0.1:8888/search?q=thu+do+nuoc+Uc&format=json" -Headers @{ "X-Real-IP" = "127.0.0.1" }).results | Select-Object -First 3 title, url
```

SearXNG chỉ chạy khi app đang mở; đóng app rồi kiểm tra thì "không chạy" là đúng.

### Xử lý sự cố

| Hiện tượng | Cách xử lý |
|---|---|
| Script báo không tìm thấy `py`/`python` | Cài Python 3.10+ từ python.org, mở terminal mới rồi chạy lại |
| App mở nhưng `/healthz` không trả `200` | Kiểm tra `.local/searxng-venv/Scripts/python.exe` có tồn tại; chạy lại script cài |
| Cổng 8888 bị chương trình khác chiếm | Tắt chương trình đó, hoặc chạy SearXNG ở cổng khác và đặt `LIM_SEARXNG_URL` |
| Chạy tay để xem log lỗi (đóng app trước) | Từ thư mục dự án: `cd .local\searxng-src; $env:SEARXNG_SETTINGS_PATH = (Resolve-Path ..\searxng-settings.yml).Path; ..\searxng-venv\Scripts\python.exe -m searx.webapp` |

Ghi chú: SearXNG không hỗ trợ Windows chính thức; script xử lý hai điểm riêng của Windows (bỏ vài tệp mẫu Linux có `:` trong tên, thêm module `pwd` thay thế trong venv). Nếu sau này dùng Docker, chạy image `searxng/searxng` ở cổng 8888 — app không cần đổi gì.
