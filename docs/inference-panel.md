# Tham số chạy model

Panel bên phải hội thoại có thể ẩn/mở bằng nút tham số trên thanh tiêu đề. Trên màn hình hẹp panel mở phủ bên phải; chat giữ bố cục hiện tại. Bản nháp được giữ khi ẩn panel.

- Context: 4096 / 8192 / 16384; là bản nháp cùng các tham số khác, lưu khi nhấn Áp dụng. Context đã lưu dùng chung với Cookbook.
- Temperature 0–2, Top P 0–1, Top K 0–100, Repeat penalty 0–2.
- Output: 128–8192 token, mặc định 2048; bao gồm reasoning.
- Seed: số nguyên 0–2147483647, hoặc bỏ trống để không gửi option seed.
- System prompt: tối đa 12000 byte UTF-8; để trống để bỏ system message.
- Nút Áp dụng lưu inference + prompt (`inference_settings`) và `context_size` bằng một transaction SQLite. Mặc định điền lại bản nháp, cần Áp dụng để lưu. Cấu hình dùng chung cho các hội thoại.

Mỗi lần `start_turn` chụp cấu hình đã lưu một lần. Ollama `/api/chat` nhận `num_ctx`, `num_predict`, `temperature`, `top_p`, `top_k`, `repeat_penalty`, tùy chọn `seed` và system message. Thay đổi cấu hình không thay đổi lượt đang chạy.

Ngân sách lịch sử tiếp tục dùng ước lượng bảo thủ theo byte UTF-8, trừ prompt/output/framing trước khi thêm memory và lịch sử. Đây không phải tokenizer của model. Nếu không giữ được tin nhắn mới nhất, runtime từ chối lượt thay vì cắt nội dung. Tăng context/output có thể dùng thêm bộ nhớ và thời gian chạy. Qwen3 vẫn giữ `think=false` theo chính sách hiện có.

Browser preview lưu inference + context ở localStorage riêng; không chạy inference. Bản desktop là nơi lưu SQLite và gọi Ollama.

Tham chiếu: https://docs.ollama.com/modelfile (tham số inference chính thức).

## Sau hội đồng UI

Dock cửa sổ thu gọn nằm trong sidebar, có tên/tooltip và nút khôi phục ngay cả khi sidebar thu gọn. Footer Áp dụng/dirty luôn hiện, chỉ phần trường cấu hình cuộn. Panel nhớ trạng thái mở/đóng qua localStorage; đóng giữ draft, nút mở có dấu dirty và composer nói rõ gửi dùng cấu hình đã lưu.

Cookbook có thể đổi context đã lưu qua luồng riêng. Panel tự đồng bộ nếu chưa chỉnh context; nếu đang có draft context khác, giữ draft và nhắc rằng Áp dụng sẽ dùng giá trị đang hiện. Snapshot và lượt chạy đọc inference + context cùng một query/DB lock.
