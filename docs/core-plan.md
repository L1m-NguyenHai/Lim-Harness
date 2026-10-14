# Lõi Lim Harness: thứ tự triển khai

Ngày nghiên cứu: 12/10/2026 (Asia/Bangkok). Mẫu: `odysseus/`, nhánh `dev`, commit `a8c147b2`.

**Kết luận:** làm runtime hội thoại trước: lưu phiên → lắp context → gọi một model thật → streaming → dừng/lỗi → lưu kết quả. Tiếp theo mới thêm vòng lặp tool và bộ nhớ Obsidian. Đây là đề xuất từ nghiên cứu mã, chưa triển khai hoặc chốt công nghệ backend.

## Những phần lõi tìm thấy trong Odysseus

| Trách nhiệm | Bằng chứng trong mẫu | Áp dụng cho Lim Harness |
| --- | --- | --- |
| Điểm nhận lượt chat | `routes/chat_routes.py:2406`, bridge `:4531` | Một command nhận session và nội dung; không mang toàn bộ tầng route web sang desktop |
| Lưu hội thoại | `core/database.py:177` Session, `:277` ChatMessage; `core/session_manager.py:241`, `:260` | Session/message có ID ổn định; lưu user trước khi gọi model; tránh ghi vào phiên đã xóa |
| Lắp context | `routes/chat_helpers.py:1006`; `core/models.py:164`; `src/context_budget.py:28` | Tách transcript hiển thị khỏi input model; giữ tin user mới nhất và dành dung lượng cho output |
| Adapter model | `src/llm_core.py:2900`, HTTP stream `:3452` | Chuẩn hóa text delta, tool call, kết thúc và lỗi của một provider trước |
| Vòng lặp agent | `src/agent_loop.py:21015`, vòng lặp `:25728`, gọi provider `:26899`, thực thi `:33464`, ghép tool result `:15855` | Model → tool call → executor → tool result → model; giới hạn số vòng, số tool và thời gian |
| Contract tool | `src/agent_tools/__init__.py:38`, `src/tool_schemas.py:287`, `src/tool_execution.py:1616` | Registry tối thiểu gồm ID, schema, handler và trạng thái bật/tắt |
| Contract memory | `src/memory_provider.py:33`, registry `:258` | Adapter riêng cho bộ nhớ; memory provider của mẫu không đồng nghĩa với Obsidian |

Loop mẫu có hơn 37.000 dòng và phụ thuộc nhiều tính năng ứng dụng. Học ranh giới trách nhiệm và luồng dữ liệu; không lấy nguyên loop làm nền của bản đầu.

## Mốc 1 — Hội thoại thật và bền vững

**Luồng cần hoàn thành:** gửi prompt → nhận stream thật → dừng được → đóng/mở lại còn lịch sử và trạng thái đúng → hỏi tiếp dùng context trước đó.

Đề xuất đặt runtime trong Rust của Tauri để tận dụng desktop host hiện có. Chưa cần service Python riêng khi chưa có module yêu cầu Python. Dùng command cho `start_turn` / `cancel_turn`, và IPC Channel để gửi stream về React. Tauri khuyến nghị Channel cho dữ liệu streaming và cung cấp truyền có thứ tự: [Calling Rust / Channels](https://v2.tauri.app/develop/calling-rust/).

Đề xuất SQLite cho dữ liệu bền vững:

- `sessions`: ID, tiêu đề, cấu hình model, thời gian tạo/cập nhật.
- `messages`: ID, session ID, run ID nếu có, role, nội dung, nguồn, thứ tự và thời gian.
- `runs`: ID, session ID, model, trạng thái, thời gian bắt đầu/kết thúc, lỗi.
- Trạng thái run: running, completed, cancelled, failed, interrupted. Khi mở lại ứng dụng, run còn running từ lần trước được ghi nhận interrupted.

Ràng buộc ngay từ bản đầu:

- Một run hoạt động mỗi phiên. Lưu kết quả cuối đúng một lần; xác định rõ phần trả lời đang dở khi dừng/lỗi.
- Stream có `run_id`, `session_id`, số thứ tự; UI không ghép delta của run cũ vào lượt mới.
- Hủy run phải hủy request đang chạy; timeout và lỗi được hiển thị, lưu trạng thái rõ ràng.
- Context là bản làm việc riêng: system prompt + lịch sử phù hợp + user mới nhất + output budget. Chưa cần tóm tắt tự động; không sửa transcript gốc để cắt context.
- Cấu hình provider và thông tin xác thực nằm phía native; không lưu khóa API trong state/localStorage của React.
- Phản hồi mẫu UI hiện tại trong `src/App.tsx:161` phải được phân biệt với câu trả lời thật. Khi chuyển dữ liệu cũ, không đưa các thông báo này vào assistant history gửi model.

Sự kiện tối thiểu: started, text_delta, completed, failed, cancelled. Tool events thêm ở mốc 2. Provider cụ thể còn cần chọn theo endpoint/model sẵn có.

## Mốc 2 — Agent loop nhỏ với một tool đọc ghi chú

**Luồng cần hoàn thành:** model yêu cầu `read_note` → runtime kiểm tra arguments → đọc tệp được phép → trả tool result đúng call ID → model trả lời kèm nguồn.

- Registry cố định với schema và handler; chạy tuần tự trước.
- Chỉ thực thi khi đã nhận đủ arguments của tool call, không chạy từng fragment của stream.
- Giới hạn số vòng, số tool call, thời gian mỗi tool và thời gian toàn run; ghi rõ lỗi khi vượt giới hạn.
- Ghi call ID, arguments đã kiểm tra, kết quả/lỗi và thời gian; cho UI biết tool đang chạy.
- Nội dung ghi chú là dữ liệu tham khảo có nguồn, không trở thành system instruction.

Tool đầu là cầu nối xuyên suốt để kiểm chứng harness, đồng thời tạo contract cho module sau này. Chưa cần marketplace hoặc nạp plugin động.

## Mốc 3 — Obsidian vault thực tế

- Chọn vault và đọc/tìm kiếm `.md` theo tên, đường dẫn, từ khóa; trả excerpt và nguồn.
- Giới hạn mọi đường dẫn trong vault, xử lý symlink để tránh thoát root; bỏ `.obsidian` và tệp không thuộc phạm vi.
- Context chỉ nhận phần ghi chú liên quan trong budget; không nạp cả vault vào prompt.
- Sau luồng đọc, thêm lưu ghi chú theo yêu cầu rõ ràng và kiểm tra xung đột trước khi ghi đè.
- Embedding, RAG, trích xuất memory tự động và tóm tắt dài hạn là các bước sau khi có dữ liệu và nhu cầu đo được.

Trong các đường mã đã nghiên cứu, chưa tìm thấy adapter Obsidian chuyên biệt. Phần này cần Lim Harness tự xây; không đánh dấu memory của mẫu là tích hợp vault sẵn có.

## Để sau

Routing nhiều model, fallback/teacher, MCP discovery và subprocess, tool song song, browser/shell, background scheduler, planner, media/voice/avatar, tự động compaction và hệ thống tài khoản. Chúng chưa cần để chứng minh luồng chat → tool → bộ nhớ của bản đầu.

## Trạng thái và giới hạn nghiên cứu

- Hiện Lim Harness lưu UI bằng localStorage (`src/state.ts`, `src/App.tsx:77`); backend `src-tauri/src/lib.rs` chỉ khởi động ứng dụng. Chưa có provider, session store native hoặc executor.
- Đã đọc và đối chiếu mã mẫu, mã hiện tại và tài liệu Tauri. Không chạy backend Odysseus, gọi model hoặc kiểm thử runtime trong lượt nghiên cứu này. Các luồng “cần hoàn thành” ở trên là tiêu chí tương lai, không phải kết quả kiểm tra.
- Không chỉnh sửa mẫu hoặc sao chép mã. Odysseus dùng AGPL-3.0 trong `odysseus/LICENSE`; nếu sau này kế thừa mã, cần ghi nguồn và giữ thông tin giấy phép liên quan.
- Việc đầu tiên khi triển khai: xây session store và run lifecycle native, nối một provider vào UI hiện có để hoàn thành mốc 1.
