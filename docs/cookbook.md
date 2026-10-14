# Cookbook model cục bộ

## Cách dùng

1. Mở Ollama, rồi mở Lim Harness desktop bằng `npm run tauri dev` hoặc executable đã build. Browser preview chỉ xem catalog, không tải/chạy model.
2. Mở Cookbook ở sidebar. “Làm mới” đọc CPU, RAM/VRAM trống, dung lượng ổ đĩa và các model Ollama đã cài/đang nạp. Khi trang mở, trạng thái được cập nhật khoảng 20 giây một lần lúc không thao tác.
3. Cookbook là cửa sổ nổi trong app, kéo bằng thanh tiêu đề, thu gọn về dock, phóng to/khôi phục, đóng và đổi kích thước từ góc dưới. Alt + phím mũi tên trên thanh tiêu đề di chuyển bằng bàn phím; phím mũi tên trên nút góc dưới đổi kích thước. Đóng/thu gọn không hủy thao tác tải đang chạy; mở lại để theo dõi hoặc ngắt yêu cầu.
4. Tab Download có Direct Download cho tên/tag Ollama hoặc namespace/tên:tag. Tag bỏ trống dùng :latest. Không nhận URL Hugging Face hoặc registry riêng. Scan / Download dùng bảng có lọc/sắp xếp và xem chi tiết fit/nguồn. Tải chỉ tải và xác nhận `/api/tags`, không nạp hoặc đổi model chat.
5. Tab Launch chọn model đã cài, context 4K/8K/16K rồi nạp. App xác nhận `/api/ps`, lưu model/context chung cho chat. Tab Active hiển thị model đang ở bộ nhớ và cho giải phóng; Dependencies hiển thị trạng thái/version Ollama, Settings cấu hình context. Thiếu dữ liệu được hiển thị rõ, không tạo speed/score giả.
6. Tiến độ tính theo layer hiện tại, không phải phần trăm toàn bộ job. Có thể “Ngắt yêu cầu”; app ngắt stream cục bộ nhưng Ollama có thể còn xử lý phía server. Cache tải dở được giữ để thử lại. Làm mới để xem trạng thái thực tế.
7. Nút giải phóng bên cạnh model đang nạp unload model, giữ tệp đã tải. Ollama cũng có thể tự unload sau 5 phút idle. Chat kế tiếp có thể nạp lại model đang chọn.

Native chỉ cho một thao tác setup và chặn setup/unload khi chat đang chạy. Tải trực tiếp nhận tag Ollama hợp lệ ngoài catalog; model đã có trong Ollama có thể nạp và chọn. Không tự cài/chạy dịch vụ Ollama, tự tải khi mở trang, xóa model hoặc ghi vault.

Nguồn tệp: `src/Cookbook.tsx`, `src/cookbook-api.ts`, `src/cookbook.css`, `src/model-catalog.ts`, `src-tauri/src/cookbook.rs`. Hardware dùng CIM Windows và NVIDIA `nvidia-smi` với giới hạn thời gian; thiếu dữ liệu được giữ là chưa xác định. Thư mục model lấy theo `OLLAMA_MODELS` của app hoặc mặc định người dùng; service dùng cấu hình khác có thể lưu nơi khác. Dung lượng trống hiển thị là thông tin tham khảo, lỗi ổ đĩa thực tế do Ollama xác nhận khi tải.

## Catalog và đánh giá

Catalog hiện có snapshot 67 tag từ 20 family, cùng cập nhật online và cache. [Nguồn metadata, công thức đánh giá và giới hạn](model-catalog.md).

Ba tag của catalog đầu tiên được giữ dưới đây làm ví dụ lịch sử, đã đối chiếu ngày 13/10/2026:

| Model | Download xấp xỉ | Quantization | Khả năng do model công bố |
| --- | --- | --- | --- |
| [qwen3:4b](https://ollama.com/library/qwen3:4b) | 2,5 GB | Q4_K_M | Văn bản, tools, thinking |
| [gemma3:4b](https://ollama.com/library/gemma3:4b) | 3,4 GB | Q4_K_M | Văn bản, vision |
| [qwen3:8b](https://ollama.com/library/qwen3:8b) | 5,2 GB | Q4_K_M | Văn bản, tools, thinking |

GB ở đây là số thập phân làm tròn trên trang Ollama. Backend phải dùng tổng byte thật của download khi báo tiến độ hoặc kiểm tra ổ đĩa. Tag có thể đổi; catalog chưa tự refresh. Capabilities là khả năng model, không khẳng định Lim đã có luồng gọi tool hoặc gửi ảnh.

`src/model-catalog.ts` cung cấp snapshot kèm app, type `Hardware` và hàm `getModelFit(model, hardware, contextSize, assessment)`. Catalog đã bỏ giả định ba model chung; params, quantization và byte từ metadata thật. Auto/RAM/GPU đánh giá riêng từng pool bộ nhớ trống. Các con số null là chưa biết, không phải 0. Bảng có FIT SCORE (phù hợp máy) và SPEED quan sát thực, xem công thức hiện tại trong tài liệu catalog.

Fit dùng weights × 1,10 + 0,75 GiB runtime + KV giả định theo params/context. Công thức chi tiết và ngoại lệ embedding ở `docs/model-catalog.md`. Chừa thêm ít nhất 1 GiB/20% VRAM trống và 2 GiB/20% RAM trống. Không cộng RAM với VRAM, không cộng GPU khác nhau.

`recommended` chỉ khi ước lượng vừa cả pool VRAM trống và RAM trống riêng; vẫn cần Ollama xác nhận GPU khi nạp. `caution` áp dụng khi chỉ RAM có vẻ đủ hoặc thông tin RAM chưa đủ dù VRAM vừa; không bảo đảm CPU/offload hay tốc độ. `unknown` là thiếu số đo; `limited` là cả hai pool đã biết đều không vừa. Đo lại trước khi nạp vì bộ nhớ đang trống thay đổi theo ứng dụng khác. Disk trống, runtime đã cài, tag thực tế và trạng thái nạp cần backend xác nhận riêng.

Bộ nhớ Obsidian cùng lịch sử, instruction và output đều chiếm context chung. Context 4K/8K là ngân sách tổng, không phải dung lượng ghi chú riêng. Không dùng context tối đa do model quảng bá để suy ra context vừa máy. Vision có thể cần thêm bộ nhớ ảnh/projector; ước lượng hiện phục vụ chat văn bản.

Không xếp hạng chất lượng hay dự đoán token/giây. Chưa có benchmark hoặc bảo đảm tải/nạp model thành công. Mã fit được viết mới; Odysseus chỉ được tham khảo về cách tách hardware, catalog và lifecycle.
