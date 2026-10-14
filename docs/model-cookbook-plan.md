# Cookbook model cho Lim Harness — phân tích trước triển khai

Ngày: 13/10/2026, Asia/Bangkok. **Chưa triển khai catalog, recommendation, tải hoặc nạp model tự động.**

> Bản phân tích ban đầu được giữ để theo dõi quyết định. Sau đó người dùng đã yêu cầu triển khai trang Cookbook; mã và hướng dẫn hiện tại ở `docs/cookbook.md`. Không tự tải model trong quá trình xây trang; thao tác tải bắt đầu khi người dùng chọn nút trong app.

## Khả thi và phạm vi bản đầu

Có thể tạo luồng nhận diện phần cứng → đề xuất vài model → tải bằng Ollama → nạp → chọn cho hội thoại. Dùng Ollama service hiện có và API cục bộ, không cần tạo inference server riêng. Model là tài nguyên tải từ registry, không phải một mô hình recommendation cần huấn luyện.

Cookbook nên đặt trong Cài đặt > Model, với ba lựa chọn ban đầu và lý do phù hợp. Mỗi lựa chọn hiển thị tag chính xác, dung lượng tải, quantization, khả năng, context đề xuất và trạng thái. Thông số fit là ước lượng; tốc độ/chất lượng cần đo riêng, không tự tạo điểm benchmark.

## Phần cứng đã đọc thực tế

| Thành phần | Kết quả |
| --- | --- |
| CPU | Intel Core i7-12700H, 14 core / 20 thread |
| RAM | 15,7 GiB (máy khoảng 16 GB) |
| GPU | NVIDIA RTX 3060 Laptop GPU, 6.144 MiB VRAM |
| VRAM còn trống tại lần đọc | 6.023 MiB; có thể thay đổi khi chạy ứng dụng khác |
| Ổ C còn trống | Khoảng 383,6 GiB |
| Ollama | Service phản hồi tại localhost:11434; `/api/tags` trả `models: []` |

Bằng chứng: truy vấn CIM Windows, `nvidia-smi` và API tags. Không đọc dữ liệu vault cá nhân; chưa tải hoặc chạy model.

## Danh sách đề xuất cho máy này

Đây là suy luận từ phần cứng và dung lượng bản quantized, **chưa benchmark trên máy**. Kích thước tải không bằng lượng VRAM khi chạy: còn KV cache, context và runtime overhead.

| Model | Bản trên Ollama / kích thước công bố | Đề xuất |
| --- | --- | --- |
| [qwen3:4b](https://ollama.com/library/qwen3:4b) | Q4_K_M, khoảng 2,5 GB; tools/thinking, đa ngôn ngữ | Lựa chọn mặc định cho chat + memory + chuẩn bị tool; bắt đầu context 4K |
| [gemma3:4b](https://ollama.com/library/gemma3:4b) | Q4_K_M, khoảng 3,4 GB; vision | Lựa chọn thay thế khi cần đọc ảnh; lõi hiện tại chỉ gửi text, chưa triển khai vision |
| [qwen3:8b](https://ollama.com/library/qwen3:8b) | Q4_K_M, khoảng 5,2 GB; tools/thinking | Nhóm “cần cân nhắc”: sát VRAM sau khi thêm cache/overhead; có thể offload CPU và chậm hơn |

Tag có thể được cập nhật trên registry. Catalog phải ghi ngày nguồn/digest khi có, làm mới metadata trước tải. Không gọi model 4B là bảo đảm vừa mọi cấu hình; xác nhận thực tế sau nạp. Không cộng RAM hệ thống với VRAM như một pool thống nhất.

## Điều học được từ cookbook Odysseus

Mẫu `dev` / `a8c147b2`, chỉ nghiên cứu:

- `services/hwfit/hardware.py:568`, `:792`: Windows CIM/PowerShell và NVIDIA `nvidia-smi`; detection/cache.
- `services/hwfit/models.py:192`, `fit.py:354`, `:431`, `:708`: weights + KV + overhead, fit theo context và xếp hạng.
- `static/js/cookbook-hwfit.js:648`: nhánh Ollama ước lượng khoảng 0,6 GB/B, thiếu context/KV và không có tốc độ thực. Không dùng nguyên heuristic này làm lời bảo đảm.
- `services/hwfit/data/README.md:3`: catalog shipped rỗng; cần danh mục riêng có nguồn, không có catalog hoàn chỉnh để kế thừa ngay.
- `routes/cookbook_routes.py:1106`, `:2019`: download và serve; `static/js/cookbook-hwfit.js:1767`: model chưa tải thì bắt đầu download rồi yêu cầu bấm Run lần nữa.

Lim có thể làm một chuỗi liên tục tải → nạp → chọn model, với trạng thái rõ từng bước. Không sao chép mã AGPL của mẫu trong lượt này.

## Luồng kỹ thuật đề xuất

1. Native đọc hardware snapshot (RAM available, từng GPU/VRAM, dung lượng ổ chứa model). CIM không rõ VRAM thì dùng `nvidia-smi`; không lấy AdapterRAM 32-bit làm số liệu duy nhất. GPU được nhận diện chưa chứng minh Ollama đang dùng GPU đó.
2. Catalog nhỏ được quản lý trong dự án, gồm tag, capability, quantization, kích thước, metadata kiến trúc nếu biết, nguồn/ngày. Đánh giá fit có headroom, output reserve và context vault. Thiếu metadata thì ghi “ước lượng/chưa xác định”.
3. Nút “Tải và dùng” gọi `/api/pull`, stream tiến độ từng layer; không cộng phần trăm giữa các layer hoặc báo thành công trước trạng thái cuối. Cho hủy phía client, nhưng cần xác minh semantics hủy của server trước khi tuyên bố download đã dừng hoàn toàn. Tải lại có thể tận dụng cache; không tự xóa blob dùng chung.
4. Xác nhận `/api/tags` thấy đúng model sau tải. Nạp bằng request rỗng, với context đã chọn và keep_alive hữu hạn; xác nhận `/api/ps` thấy model. Cấu hình context dùng để nạp phải khớp lượt chat để tránh reload tăng bộ nhớ.
5. Sau khi ready mới chọn model cho chat. Thất bại/OOM giữ lại model đã tải để có thể giảm context và thử lại. Phân biệt service offline, chưa cài runtime, lỗi tải, lỗi nạp và lỗi inference. Profile cần cấu hình thinking theo khả năng model; lõi hiện gửi `think:false` riêng với tag `qwen3` / `qwen3:*` để ngân sách 1.024 token không bị dùng hết trước câu trả lời. Chưa xác nhận profile này bằng inference thực tế.

Trạng thái: available → downloading → installed → loading → ready; các nhánh cancelled/failed. Sẵn sàng là trạng thái có thời hạn vì Ollama có thể unload sau khi idle; nên làm mới `/api/ps` khi cần. Lệnh dừng inference không đồng nghĩa với unload model.

Tài liệu primary: [Ollama API pull](https://docs.ollama.com/api/pull), [API tags](https://docs.ollama.com/api/tags), [API ps](https://docs.ollama.com/api/ps), [FAQ preload/keep_alive và bộ nhớ](https://docs.ollama.com/faq). Trang pull được web reader trả nội dung thiếu; cần đối chiếu API đầy đủ trước khi triển khai parser tải.

## Kết hợp Obsidian

Vault là bộ nhớ ngoài, không nạp tất cả vào model. Lõi hiện đọc tìm `.md` theo từ khóa và đưa trích đoạn có nguồn vào context. Cookbook phải dành ngân sách context cho các ghi chú này; chọn 4K/8K/16K có ảnh hưởng đến KV cache. Vector search/embedding có thể thêm sau khi tìm từ khóa không đáp ứng nhu cầu đo được.

Việc tiếp theo được đề xuất: triển khai hardware snapshot + catalog ba lựa chọn, sau đó pull progress và load/readiness; chưa tự tải model trong lượt phân tích này theo yêu cầu người dùng.
