# Memory v1 — Obsidian

Markdown là nguồn chính của memory; SQLite giữ transcript, cấu hình và hàng đợi trích xuất. Không cần Git, Obsidian plugin, embedding hoặc service Python. Implementation do Lim tự xây, không sao chép hai repo tham khảo.

## Bắt đầu

**Khởi tạo nhanh:** Trong Bộ nhớ (khi chưa kết nối vault) hoặc Cài đặt → Bộ nhớ Obsidian, bấm **Khởi tạo vault mặc định**. Lim tạo `Lim Harness Memory` trong thư mục Documents của hệ điều hành, khởi tạo `LimMemory/`, kết nối và bật cả recall lẫn tự nhớ. Thao tác thử mở Obsidian qua URI; cần cài và mở Obsidian một lần để đăng ký protocol. Với vault chưa đăng ký, chọn **Open folder as vault** trong Obsidian và trỏ đến thư mục vừa tạo một lần. Lim không sửa danh sách vault riêng của Obsidian. Nút **Mở trong Obsidian** trong Cài đặt gửi yêu cầu mở `LimMemory/index.md`; việc hệ điều hành nhận yêu cầu không xác nhận Obsidian đã mở đúng note. Thư mục mặc định có dữ liệu không nhận diện được bị từ chối, không ghi đè. Luồng nhập đường dẫn bên dưới vẫn dùng được cho vault có sẵn.

1. Chạy desktop bằng `start.ps1`, chọn vault trong Cài đặt.
2. Mở **Bộ nhớ → Bật bộ nhớ cho vault**. Hai tùy chọn **Dùng memory khi trả lời** và **Tự ghi nhớ sau hội thoại** được bật; có thể tắt riêng từng tùy chọn.
3. Chat hoặc bấm **Thêm memory**. Sau câu trả lời hoàn tất, Lim xử lý memory khi model rảnh. Thông báo cập nhật chỉ hiện sau khi native xác nhận ghi.
4. Mở vault trong Obsidian, vào `LimMemory/index.md`. App có thêm/sửa/ghim/quên và xem nguồn/lịch sử.

Vault đã kết nối từ bản cũ vẫn chỉ đọc đến khi bật tính năng. Cấu hình memory lưu riêng theo vault; chỉ ghi trong `LimMemory/`. Ghi chú localStorage cũ vẫn là mục riêng, không tự di chuyển.

## Cấu trúc và chỉnh trong Obsidian

Khi bấm **Mở trong Obsidian**, Lim đọc danh sách vault trong cấu hình Obsidian theo đường dẫn chuẩn của hệ điều hành (chỉ đọc). Nếu xác nhận có vault đã đăng ký chứa kho memory, Lim mở trang tổng hợp. Nếu chưa đăng ký hoặc không đọc/xác minh được cấu hình, Lim mở **trình quản lý vault** bằng `obsidian://choose-vault` và hiển thị thư mục cần chọn qua **Open folder as vault → Open**, tránh gửi liên kết note tới vault chưa tồn tại trong Obsidian. Cấu hình Obsidian không bị Lim ghi sửa; bản portable hoặc vị trí cấu hình riêng có thể luôn đi qua trình quản lý vault.

```text
LimMemory/
  .lim-memory.json          marker schema 1
  index.md                  trang wikilink sinh lại được
  facts/<tên-ngắn>-<id>.md   một memory và lịch sử trong cùng tệp
```

Frontmatter có ID, loại (`preference`, `personal`, `work`), trạng thái (`active`, `retracted`), ghim, ngày UTC RFC3339, nguồn session/message, lịch sử và operation IDs. Nội dung hiện tại ở **Nội dung**; **Nguồn** và **Lịch sử** trong thân là bản trình bày từ metadata.

- Sửa **Nội dung** hoặc Properties `title`, `category`, `pinned`, `status`. Giữ ID, nguồn, operation IDs và lịch sử; không sửa bản trình bày Nguồn/Lịch sử để đổi metadata.
- Đổi tên/chuyển note trong `facts/` giữ cùng ID. Lim đọc lại filesystem khi truy hồi/poll. Tệp lỗi hoặc ID trùng hiện diagnostics; không tự chữa rồi ghi đè.
- Revision là SHA256 toàn tệp. App giữ bản nhập nhưng chặn lưu nếu Obsidian đã sửa; tải lại trước khi ghi.
- **Quên** rút lại memory, không xóa note/transcript. Recall không dùng note retracted. Muốn thêm lại có chủ đích có thể đổi `status` sang `active` trong Obsidian.
- `index.md` không là nguồn dữ liệu. Bật lại tính năng hoặc ghi tiếp để dựng lại; lỗi index không làm mất memory đã commit.

Không ghi đè `LimMemory/` có sẵn nhưng không nhận diện được marker. Nếu khởi tạo gián đoạn trước khi marker hoàn tất, kiểm tra thư mục trước khi phục hồi; Lim không tự nhận quản lý thư mục lạ.

## Tự nhớ, job và ghi an toàn

Mỗi câu trả lời `completed` tạo job trong cùng transaction lưu kết quả. Failed/cancelled/truncated chưa tạo job; Continue hoàn tất có thể tạo job nếu vault/epoch của lượt gốc vẫn hợp lệ. Job giữ model/context của lượt đó và tối đa ba cặp hội thoại; chỉ lời user mới nhất được dùng làm bằng chứng mới.

Extractor gọi Ollama JSON schema, không streaming, temperature 0, tối đa 1.024 output tokens; trả tối đa ba create/update/retract. Native kiểm tra schema, ID trong tập đã cung cấp, quote nguyên văn, nguồn user, kích thước và dấu hiệu credential phổ biến; không nhận đường dẫn từ model. Đây không phải bộ kiểm chứng ngữ nghĩa hoặc phát hiện mọi loại bí mật. Không đưa thông tin xác thực vào hội thoại cần ghi nhớ.

Ở v1, nội dung tự động được giữ nguyên từ quote user đã xác minh; không dùng bản diễn giải/dịch của model làm dữ liệu gốc. Với tin ngắn, schema ràng buộc quote là toàn bộ lời user để tránh model sửa dấu/hoa/thường. Model vẫn chọn có cần nhớ, loại, tiêu đề và memory cần cập nhật. Có thể sửa gọn thủ công; việc chọn điều đáng nhớ và trả lời từ memory vẫn phụ thuộc chất lượng model.

Model không chắc có thể trả không có thao tác. Nội dung trùng sau chuẩn hóa bị bỏ qua. Quên qua chat cần yêu cầu trực tiếp và khớp rõ một memory; trường hợp mơ hồ báo chọn trong cửa sổ Bộ nhớ. Thao tác thủ công không phụ thuộc khả năng extraction của model.

Worker tuần tự, chỉ chạy khi chat/Cookbook không dùng model. Chat/Continue/Cookbook mới hủy inference nền và giữ job để xử lý khi rảnh. JSON/HTTP/timeout/schema lỗi chỉ làm job failed, không làm câu trả lời đã lưu thành failed. Xem/thử lại job trong Bộ nhớ; running trở về pending khi mở app lại.

Đổi vault, thay cấu hình memory hoặc sửa thủ công qua app làm job cũ obsolete. Xóa phiên hủy job của phiên; memory đã ghi vẫn riêng, dùng Quên để rút lại. Baseline revision/trạng thái ngăn job cũ ghi đè sửa/rút lại ngoài app. Baseline job mới được cập nhật sau commit của chính Lim, không bỏ qua chỉnh sửa Obsidian.

Một writer trong tiến trình, kiểm tra hash trước replace. Mỗi thao tác là một file atomic có operation ID chống retry trùng. Batch nhiều memory không phải transaction filesystem: lỗi sau một phần giữ các note đã commit và báo số thay đổi/lỗi; retry tiếp từ operation IDs. Windows sync tệp tạm và replace write-through.

## Recall và nguồn

System prompt mặc định: `You are a helpful assistant.` Không thêm tên Lim, model tag hoặc quy tắc phong cách/ngôn ngữ vào system context. Prompt mặc định cũ được chuẩn hoá sang câu mới khi đọc cấu hình; prompt tự chỉnh được giữ nguyên. Khi user để prompt rỗng thì không gửi system role ở lượt mới. Hướng dẫn xử lý nguồn vẫn đi cùng context truy hồi memory.

Search ghi chú thông thường bỏ toàn bộ `LimMemory/`. Recall chỉ gửi nội dung hiện tại và loại của note active vào context; không gửi nguồn/lịch sử/index. Xếp hạng dùng tiêu đề, nội dung và keyword trong quote nguồn hiện tại để tìm lại bằng lời user. Keyword chuẩn hóa hoa/thường, dấu và `đ/d`; category có một số từ khóa để tìm fact ngắn. Đây vẫn là lexical retrieval, chưa hiểu mọi câu hỏi đồng nghĩa.

Core ghim dùng tối đa một phần tư budget memory; phần còn lại cho memory liên quan và ghi chú. Tổng budget tối đa 4.500 byte UTF-8, không cắt giữa bản ghi. Nguồn JSON được ghép cùng câu hỏi mới nhất trong một user turn; transcript gốc giữ nguyên. Memory là dữ liệu tham khảo không đáng tin về instruction, không cấp quyền chạy tool.

Nguồn thực sự gửi model được lưu cùng assistant: path/title/excerpt và memory ID/revision nếu có. UI hiện nguồn cả khi model bỏ `[vault:N]` trong text. Continue kiểm tra nguồn memory còn active, đúng revision, cùng vault và đang được phép dùng; nếu đổi thì yêu cầu hỏi lại thay vì đưa nguồn đã quên vào prompt.

Scan memory giới hạn 5.000 entries, 1.000 Markdown files, 256 KiB/tệp, 8 MiB tổng và khoảng hai giây giữa các lần đọc. Chỉ UTF-8, bỏ hidden/dependency folders, symlink và Windows reparse point. Root/path canonicalized. Scan thiếu hoặc có lỗi chặn ghi; file lỗi bị loại khỏi recall và hiện diagnostics. Filesystem call treo không bị ngắt bởi time budget; kiểm tra đường dẫn không phải sandbox chống một tiến trình liên tục thay đường dẫn.

## Kiểm tra và giới hạn xác minh

```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib
npm run build
npm run test:state
npm run test:context
npm run test:thinking
npm run test:colors
```

Fixture native với model thật, chỉ ghi vault tổng hợp tạm:

```powershell
$env:LIM_MEMORY_LIVE_MODEL = 'gemma3:1b'
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib live_native_stream_queue_and_cross_session_sources -- --ignored --nocapture
```

Test `live_ollama_fixture_extraction_and_recall` kiểm tra chất lượng nghiêm hơn, yêu cầu cả citation trong text. Trong lần thử 14–15/10/2026, gemma3:1b có lần bỏ citation; deepseek-r1:1.5b có lần không trích xuất preference. Không đồng nhất chất lượng model với nguồn metadata runtime lưu; một fixture không chứng minh chất lượng tổng thể.

Debug build hỗ trợ `LIM_HARNESS_TEST_DATA` là thư mục tuyệt đối cho database smoke test riêng; release không đọc biến này. Native runtime/model thật được kiểm tra với vault tổng hợp; chưa xác minh vault cá nhân hoặc CRUD bằng chuột trên desktop vì automation bị từ chối truy cập cursor. Web preview chỉ xác minh bố cục light/dark.

Xem [nghiên cứu ngày 14/10/2026](memory-research-2026-10-14.md). Embedding, scope dự án, semantic dedup và truy vấn lịch sử theo ngày chưa thuộc v1.
