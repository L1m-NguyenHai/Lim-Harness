# Danh sách model và đánh giá theo máy

## Nguồn dữ liệu

Snapshot kèm app hiện có **67 tag từ 20 family**, thu thập ngày 13/10/2026. Có chat, coding, vision và embedding. Tệp `src/data/model-catalog.json` chứa tag thật, nguồn và thời điểm, dung lượng byte, quantization, params, context được công bố và digest. Không tạo tag từ tên chip kích thước, không mặc định mọi model là Q4.

- Danh sách family/tag từ [Ollama Library](https://ollama.com/library) và trang `/library/<family>/tags`.
- Kích thước từ descriptor manifest trên registry Ollama; quantization và params từ config nhỏ. Chỉ đọc config có descriptor nhỏ và digest SHA-256 đúng, không tải weights/projector blobs.
- Config có thể redirect đến CDN Ollama: chỉ chấp nhận HTTPS, bucket/đường dẫn config đã xác nhận và giới hạn 64 KiB. Không ghi signed URL vào nhật ký.
- Model đã cài/đang nạp ghép từ Ollama local `/api/tags`, `/api/ps`.
- Native “Cập nhật catalog” đọc online có giới hạn, cache SQLite 24 giờ; lỗi mạng giữ cache hoặc snapshot cũ. HTML/registry không phải API catalog ổn định; thay đổi cấu trúc sẽ báo cảnh báo thay vì tự đoán metadata.

Snapshot được tạo bằng `npm run catalog:refresh` (script `scripts/refresh-model-catalog.mjs`). App đọc online bằng `src-tauri/src/catalog.rs`. Giới hạn hiện tại khoảng 20 family / 80 tag, không phải toàn bộ Library và không phải catalog Hugging Face/llama.cpp. Metadata thiếu được giữ là chưa xác định/partial.

## Các cột đánh giá

- **FIT / EST. GiB:** ước lượng bộ nhớ theo weights thực, overhead, KV giả định và context đang chọn. Auto ưu tiên GPU vừa máy, sau đó xem khả năng RAM/offload; RAM/GPU chỉ đánh giá pool được chọn. Không cộng hai pool thành một.
- **PARAMS / QUANT:** config registry hoặc metadata model đã cài. Đây là tổng params được nguồn báo, không phải active params của MoE.
- **CTX:** context đang chọn/đang nạp; mức tối đa công bố xem trong chi tiết. Public K được quy đổi 1.024 token; metadata này không có nghĩa máy chạy nổi toàn bộ context tối đa.
- **FIT SCORE:** `clamp(50 × ngân sách bộ nhớ / yêu cầu ước lượng, 0, 100)`. 50 tương ứng sát giới hạn đã chừa dự phòng; 100 có ít nhất gấp đôi yêu cầu. Context vượt mức công bố cho điểm 0. Đây là điểm phù hợp bộ nhớ, **không phải chất lượng model**.
- **SPEED:** quan sát mới nhất của lượt chat thành công với cùng model/context, tính `eval_count × 1e9 / eval_duration` từ Ollama. Bao gồm token eval của model theo cách Ollama đếm; không gồm thời gian nạp/prompt và không phải benchmark so sánh chuẩn hóa. Chưa có dữ liệu hiển thị `—`. Quan sát lưu trong SQLite setting `model_observations`.

## Công thức bộ nhớ và giới hạn

Weights × 1,10 + 0,75 GiB runtime + KV ước lượng. KV chat ở 4K được giả định `clamp(0,15 × params tỷ, 0,25, 8) GiB`, tăng theo context; thiếu params dùng 2 GiB/4K. Embedding dùng allowance 0,25 GiB/4K cho activation/batch, không gọi đó là KV decoder chính xác. Chừa ít nhất 1 GiB/20% VRAM trống hoặc 2 GiB/20% RAM trống.

Đây là heuristic bảo thủ, chưa suy từ số layer/head, GQA, loại KV hoặc cấu trúc MoE/hybrid, nên có thể dư hoặc thiếu so với runtime thật. Fit có lý do và giả định trong chi tiết; không phải lời bảo đảm nạp thành công. Giới hạn scan và ngày cache luôn được hiển thị. Context còn chứa lịch sử và trích đoạn Obsidian; không nạp cả vault.

Model embedding có thể tải để chuẩn bị memory, nhưng nút Run chat bị tắt khi catalog nhận diện embedding. Executor embedding và vector index chưa triển khai. Không tự tải model hoặc tự chạy benchmark khi cập nhật catalog.

## Đối chiếu Odysseus

Odysseus ghép HF collections/runtime cache và Ollama Library. Các dòng ENGINE llama.cpp trong ảnh thuộc hệ khác, không tự chuyển được thành tag Ollama. Speed/score ở mẫu là heuristic (`services/hwfit/fit.py`); Lim dùng score phù hợp bộ nhớ được giải thích và chỉ hiện speed đã quan sát. Mã mới được viết riêng, không sao chép code Odysseus.
