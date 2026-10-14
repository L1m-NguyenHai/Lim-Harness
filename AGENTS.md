# Lim Harness — Quy tắc dự án

## Bối cảnh và phạm vi

- Lim Harness là phần triển khai harness trong đồ án tốt nghiệp: bộ nhớ Obsidian và khả năng mở rộng qua module.
- Dự án đang được xây lại từ đầu. Chọn công nghệ theo yêu cầu thực tế; không coi công nghệ hoặc mã của bản cũ là quyết định đã chốt.
- Thư mục `odysseus/` là mẫu tham khảo được giữ nguyên. Chỉ đọc để nghiên cứu; chỉ sửa, di chuyển hoặc xóa khi người dùng yêu cầu trực tiếp.
- Khi kế thừa mã từ Odysseus, ghi nguồn và giữ thông tin giấy phép cần thiết. Phân biệt phần kế thừa với phần Lim Harness tự xây.

## Lưu tiến độ vào Notion

- Trang tiến độ chính: [Lim Harness — Theo dõi tiến độ](https://app.notion.com/p/3f7cf6346c8581e0be15fda0adceca12).
- Page ID: `3f7cf634-6c85-81e0-be15-fda0adceca12`.
- Tài liệu tổng thể: [Đồ án tốt nghiệp](https://app.notion.com/p/3f3cf6346c85814d9564f15fc0f766a4).
- Người dùng đã cho phép cập nhật tiến độ dự án trên trang này; không hỏi lại cho các cập nhật tiến độ thông thường.
- Mặc định giao việc đọc và ghi Notion cho sub-agent thư ký dùng Luna (`gpt-6-luna`) theo yêu cầu người dùng; tái sử dụng agent thư ký đang có nếu phù hợp.
- Trước khi tiếp tục một công việc đáng kể, đọc trang tiến độ để nắm trạng thái và việc tiếp theo. Đối chiếu với mã nguồn hiện tại khi có khác biệt.
- Sau mỗi mốc đáng kể và trước khi kết thúc lượt thực hiện có thay đổi dự án, cập nhật Notion: checklist liên quan, trạng thái hiện tại, việc tiếp theo và nhật ký. Các lượt chỉ hỏi đáp không cần tạo nhật ký.
- Đọc lại trang trước khi ghi; sửa đúng mục cần cập nhật và giữ nội dung khác. Gộp thông tin cùng một đợt làm việc để tránh nhật ký trùng lặp.
- Mỗi bản cập nhật ghi ngắn gọn: ngày, công việc/kết quả, bằng chứng hoặc tệp/commit/PR liên quan, kiểm tra đã thực hiện, quyết định mới, vướng mắc và bước tiếp theo. Bỏ các trường không áp dụng.
- Dùng ngày giờ theo `Asia/Bangkok`; trên Notion dùng date mention khi công cụ hỗ trợ.
- Chỉ đánh dấu hoàn thành khi có kết quả thực tế. Phân biệt đã triển khai, đã kiểm tra và đề xuất; không tự đặt phần trăm tiến độ, deadline hoặc kết quả kiểm thử.
- Không đưa token, mật khẩu, nội dung `.env` hoặc dữ liệu riêng không cần thiết lên Notion.
- Nếu Notion không truy cập được, tiếp tục phần công việc độc lập; lưu bản cập nhật chưa đồng bộ vào `docs/notion-pending.md` và báo rõ trong câu trả lời cuối. Khi truy cập được, đồng bộ các mục còn chờ và đánh dấu đã đồng bộ. Không tuyên bố đã lưu Notion nếu công cụ chưa xác nhận thành công.

## Sub-agent

- Được tạo sub-agent khi công việc có thể tách độc lập hoặc cần một lượt nghiên cứu/review riêng. Việc nhỏ làm trực tiếp.
- Giao rõ mục tiêu, phạm vi tệp và kết quả cần trả về; tránh nhiều agent cùng sửa một tệp.
- Sub-agent tuân thủ quy tắc dự án và báo kết quả, bằng chứng, kiểm tra, điểm chưa chắc chắn cho agent chính.
- Agent chính chịu trách nhiệm tích hợp, kiểm tra và tính chính xác của tiến độ; thư ký Luna thực hiện đọc/ghi Notion theo nội dung agent chính đã duyệt. Chỉ một agent ghi trang tiến độ tại một thời điểm.
- Khi tạo thư ký Luna, dùng lịch sử mới (`fork_turns="none"`) và chỉ gửi bối cảnh cần thiết: page ID, quy tắc Notion, yêu cầu đọc hoặc nội dung cập nhật đã duyệt. Không chuyển toàn bộ lịch sử chat hay mã nguồn.
- Khi đọc, thư ký trả bản tóm tắt ngắn về trạng thái, việc tiếp theo và vướng mắc; không trả toàn bộ trang trừ khi cần đối chiếu. Khi ghi, thư ký đọc lại trang, sửa đúng mục, xác nhận công cụ thành công và kiểm tra lại khi thay đổi cấu trúc.
- Thư ký không tự suy diễn kết quả triển khai/kiểm tra. Nếu có mâu thuẫn hoặc thiếu bằng chứng, báo agent chính trước khi ghi; nếu Notion không truy cập được, lưu cập nhật chờ theo quy tắc ở trên.

## Cách triển khai

- Làm một luồng hoàn chỉnh có thể dùng được trước khi mở rộng. Ưu tiên giải pháp đơn giản đáp ứng yêu cầu hiện tại.
- Ghi quyết định kiến trúc có ảnh hưởng đến các bước sau vào Notion, kèm lý do ngắn gọn; ghi rõ phần còn là đề xuất.
- Kiểm tra phù hợp với thay đổi và báo đúng những gì đã chạy; không coi kế hoạch kiểm tra là kết quả đã kiểm tra.
- Kết thúc bằng kết quả, giới hạn/vướng mắc nếu có và trạng thái lưu tiến độ Notion.
