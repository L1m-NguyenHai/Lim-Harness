# Lim Harness

AI harness desktop xây bằng Tauri v2, React, TypeScript và Rust. Odysseus được giữ nguyên trong `odysseus/` làm tài liệu tham khảo; implementation mới không sao chép mã từ dự án đó.

## Khởi động nhanh trên Windows

Sau khi clone, mở PowerShell trong thư mục dự án:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\start.ps1
```

Script kiểm tra môi trường, cài dependency từ lockfile khi cần, rồi chạy desktop. Dùng `-Mode Web` để chỉ xem frontend, `-Mode Setup` để chuẩn bị dependency, `-Mode Build` để build debug executable. Chỉ kiểm tra môi trường: `powershell -NoProfile -ExecutionPolicy Bypass -File .\check.ps1`.

Chi tiết công cụ cần cài, Git push lần đầu và cách cùng làm: [Hướng dẫn setup](docs/getting-started.md). Ollama/model/vault được cấu hình riêng trên mỗi máy; không cần Odysseus để chạy Lim.

## Chạy frontend

```sh
npm ci
npm run dev
```

Mở http://localhost:1420. `npm run build` kiểm tra TypeScript và build frontend. Node 22.18+ hoặc 24 chạy `npm run test:state` để kiểm tra đọc dữ liệu lưu, ngày không hợp lệ và lỗi localStorage.

`npm run test:colors` kiểm tra contrast chữ của các token sáng/tối. UI dùng font hệ thống, không tải font từ mạng.

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
- Chọn đường dẫn vault tuyệt đối trong Cài đặt. Adapter chỉ đọc `.md`, tìm từ khóa có giới hạn, đưa trích đoạn có nguồn vào chat; xem và tìm riêng tại Bộ nhớ. Không ghi hoặc sửa vault. Chi tiết: [memory adapter](docs/obsidian-memory.md).
- Context mặc định 4K, có thể chọn 8K/16K. Budget UTF-8 bảo thủ có thể yêu cầu rút gọn prompt; transcript gốc vẫn được giữ.
- Cookbook có tab Active/Launch/Download/Dependencies/Settings và bảng model compact. Đọc cấu hình Windows, gợi ý theo RAM/VRAM, tìm/lọc/sắp xếp catalog và model đã cài. Download tải tag Ollama và báo tiến độ từng layer; Launch nạp/xác nhận trước khi chọn cho chat. Có ngắt yêu cầu và giải phóng model. [Hướng dẫn Cookbook](docs/cookbook.md).
- Catalog kèm app có 67 tag từ 20 family, metadata registry đã xác minh và cache/cập nhật online. Bảng đánh giá Auto/RAM/GPU, FIT SCORE phù hợp bộ nhớ và SPEED quan sát từ lượt chat khi có dữ liệu. [Nguồn và công thức](docs/model-catalog.md). `npm run catalog:refresh` thu thập snapshot metadata mới, không tải weights.

## Giao diện hiện tại

- Hội thoại, tạo/chuyển phiên, tìm phiên và ghi chú, đính kèm tệp .txt/.md, sao chép tin nhắn.
- Tạo/sửa ghi chú cục bộ; chọn module mẫu; cấu hình Ollama/context/vault và sáng/tối.
- Bố cục lấy cảm hứng từ Odysseus: sidebar thu gọn, lịch sử phiên ngay bên trái, chat rộng và composer gọn. Điều hướng gồm Hội thoại, Bộ nhớ, Cookbook, Module và Cài đặt.
- Cookbook, Bộ nhớ, Module và Cài đặt mở thành cửa sổ nổi trong app: kéo, đổi kích thước, thu gọn về dock và phóng to/khôi phục. Chat là vùng chính; chọn Hội thoại thu gọn các cửa sổ đang mở. Đóng cửa sổ không hủy công việc nền.
- Logo hình học riêng dùng trong UI, favicon và icon Tauri; vector gốc tại `public/logo.svg` và `public/app-icon.svg`.
- Ghi chú cục bộ và tùy chọn giao diện dùng localStorage; hội thoại native có SQLite làm nguồn chính.
- Chưa có module executor hoặc agent tool loop. Module là danh mục UI mẫu, không được chạy bởi lõi hội thoại.

Chưa xác nhận inference thực tế trong lượt xây lõi vì Ollama trên máy chưa có model. Build thành công không đồng nghĩa với đã kiểm tra đầy đủ luồng desktop/model/vault.

Thiết kế: `design-system/lim-harness/MASTER.md`. Quy tắc agent: `AGENTS.md`. Tiến độ: [Notion](https://app.notion.com/p/3f7cf6346c8581e0be15fda0adceca12).

## Tham số chạy model

Chat có panel inference + system prompt bên phải, ẩn/mở từ thanh tiêu đề. Nhấn **Áp dụng** để lưu cấu hình dùng chung vào SQLite; lượt đang chạy giữ cấu hình lúc gửi. Context được lưu cùng lần Áp dụng, dùng chung với Cookbook. Xem `docs/inference-panel.md` cho phạm vi tham số và ngân sách context.
