# Bộ nhớ Obsidian đọc từ vault

**Contract hiện tại:** [Memory v1](memory-v1.md) đã bổ sung kho Markdown, ghi có lịch sử và tự trích xuất. Phần dưới ghi lại adapter chỉ đọc ban đầu; ghi chú ngoài `LimMemory/` vẫn chỉ đọc.

Adapter đầu tiên đọc Markdown từ một thư mục do người dùng chọn. Ứng dụng không tự tìm vault và không sửa ghi chú. Khi bật bộ nhớ cho một lượt chat, native backend tìm các từ khóa trong câu hỏi, lấy tối đa bốn đoạn ghi chú và đưa vào context với nguồn `[vault:N]` cùng đường dẫn tương đối. Nội dung ghi chú được đánh dấu là dữ liệu tham khảo không đáng tin cậy; không được coi là quyền thực thi hay chỉ dẫn hệ thống.

API Rust trong `src-tauri/src/vault.rs`: `validate_vault_path`, `search_vault` và `memory_context`. Kết quả tìm kiếm trả `hits` gồm `path`, `title`, `excerpt`; `scannedFiles`, `skippedFiles` và `truncated` cho biết giới hạn hoặc ghi chú bị bỏ qua.

Chỉ đọc tệp `.md` UTF-8. Bỏ qua thư mục/tệp ẩn, `.obsidian`, `node_modules`, `__pycache__`, `venv`, symlink và reparse point Windows. Đường dẫn được canonicalize và kiểm tra nằm trong vault. Đây là kiểm tra filesystem thông thường, chưa phải sandbox chống một tiến trình khác liên tục thay đường dẫn trong lúc đọc.

Mỗi lần tìm kiếm giới hạn 5.000 entry, 1.000 tệp Markdown, 256 KiB mỗi tệp, 8 MiB tổng nội dung và khoảng hai giây giữa các thao tác đọc. Giới hạn thời gian không thể ngắt một filesystem call đang bị treo. Không cam kết tìm toàn bộ vault nếu đạt giới hạn; UI cần hiển thị `truncated`. Các lỗi đọc thư mục làm lượt tìm kiếm báo lỗi; lỗi ở một tệp khiến tệp đó bị bỏ qua.

Tìm từ khóa hiện là so khớp chuỗi chữ thường, ưu tiên từ khóa ở tên/đường dẫn. Chưa có embedding, đồng bộ Obsidian, phân tích wikilink/frontmatter hay trích xuất bộ nhớ tự động. Context và đoạn trích bị giới hạn; khi cần tra cứu đầy đủ nên mở ghi chú gốc. Việc bật bộ nhớ sẽ gửi các đoạn được chọn tới provider của lượt chat, kể cả provider ở máy khác.

Mã adapter được viết mới cho Lim Harness. Tham khảo cách Odysseus tách dữ liệu bộ nhớ khỏi instruction và bỏ qua thư mục ẩn; không sao chép implementation.

Nghiên cứu tiếp theo về recall, nguồn bằng chứng, lịch sử fact và luồng ghi có xác nhận: [So sánh Odysseus và Obsidian Memory for AI — 14/10/2026](memory-research-2026-10-14.md). Các phương án trong báo cáo là đề xuất, chưa thay đổi contract chỉ đọc ở trên.
