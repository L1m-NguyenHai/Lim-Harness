# Hội đồng đánh giá UI — 14/10/2026

## Kết quả sửa lỗi

Ảnh người dùng cho thấy dải trống dưới chat/panel, dock nằm trong dải đó. Đo browser preview trước sửa: viewport 720px, app bottom 720px, document scrollHeight 789px. Một nhãn `.sr-only` absolute của system prompt nằm ngoài viewport, trong khi panel chưa làm containing block; nội dung cuộn có thể làm document tràn.

Sửa `src/styles.css`: khóa overflow của html/body/root, khai báo một grid row `minmax(0, 1fr)`. Sửa `src/inference.css`: panel position relative + overflow hidden, form overscroll contain. Cuộn nội dung vẫn nằm ở conversation/form/window body.

Sau sửa, mở Cookbook và Bộ nhớ rồi trở về chat để thu gọn cả hai: viewport/body/document/app bottom đều 720px, panel vẫn cuộn được. Ảnh `artifacts/ui/viewport-gap-fixed-dock.png`. Frontend build và native debug build đạt; executable `src-tauri/target/inference-ui/debug/lim-harness.exe`. Không xác nhận viewport native qua browser; native mới chỉ build. Không chạy model hoặc thêm test suite.

## Cơ chế hội đồng

Ba subagent A, B, C đọc ảnh và mã nguồn độc lập, không sửa code. Vòng 1: mỗi chủ đề có hai người bảo vệ hướng hiện tại và một người phải lập luận đối lập bằng bằng chứng. Vòng 2: root chuyển lập luận của các bên cho từng thành viên; từng người trả lời trực tiếp, nêu điểm được thuyết phục và bất đồng còn lại. Đây là vai trò phản biện được giao có chủ đích, không phải một khảo sát ba ý kiến hoàn toàn không định hướng. Không ép đồng thuận, không gán cho người khác sự đồng ý khi họ chưa nói.

| Chủ đề | Bảo vệ ở vòng 1 | Phản biện bắt buộc |
| --- | --- | --- |
| Mật độ chữ, mono, phong cách console | A, B | C |
| Cửa sổ nổi và dock | B, C | A |
| Panel inference và hợp đồng lưu | A, C | B |

## 1. Mật độ và khả năng đọc

- A/B: console đen/cyan và mono phù hợp công cụ local model, phân cấp rõ; line-height 1.85 giúp khoảng dòng. Giữ bản sắc, sửa cỡ chữ nhỏ.
- C phản biện: CSS message 12px, nav 11px, hint 10px; vùng prose tiếng Việt không cần canh cột. Đề xuất sans 14px cho chat/system prompt và hint 12px, giữ mono cho dữ liệu kỹ thuật.
- Phản hồi: A chấp nhận sans cho prose như hướng thử; B chỉ chấp nhận tăng cỡ chữ trước, chưa có bằng chứng mono làm đọc chậm. C nhận lập luận của A/B: tăng chữ là P1 có bằng chứng chắc hơn; sans là P2 cần so sánh transcript.
- Kết quả: cả ba đồng ý tăng nội dung lên 14px và hướng dẫn quan trọng lên 12px. Chọn sans hay mono còn bất đồng. Chưa đo contrast hay có nghiên cứu người dùng.

## 2. Cửa sổ nổi và dock

- B/C: giữ cửa sổ kéo/thu/phóng theo yêu cầu, sidebar là điểm mở cố định, dock có tên và ARIA khôi phục; không cần biến thành tab truyền thống.
- A phản biện: dock `fixed bottom:8px; left:50%; z-index:2000` không chừa chỗ, ảnh sau sửa cho thấy nó che hint/footer dưới composer. ARIA không giải quyết chồng lớp. Đưa dock về sidebar/rail hoặc hàng có điều kiện.
- Phản hồi: B đồng ý bỏ vị trí giữa đáy, muốn thử sidebar trước nhưng không loại hàng có điều kiện. C được A thuyết phục ưu tiên rail/sidebar; rút lại mô tả che input/Send vì ảnh chỉ chứng minh che hint. A cũng nhận giới hạn bằng chứng này.
- Kết quả: 3/3 giữ mô hình cửa sổ nổi và đồng ý đổi vị trí dock. Ưu tiên sidebar/rail; hàng có điều kiện là phương án dự phòng, không tạo dải trống thường trực. Chưa triển khai đề xuất này trong đợt sửa gap.

## 3. Panel inference và cách lưu

- A/C: giữ panel gần chat và draft + Apply; nhãn, số/slider, giới hạn prompt và trạng thái dirty hỗ trợ chỉnh nhiều tham số trước khi lưu.
- B phản biện: `inferenceOpen=true`, panel 300px mở mọi lần; tại 720px Apply/system prompt nằm dưới cuộn. Context lưu ngay trong cùng form, phần khác cần Apply: dễ gửi khi chỉ một phần cấu hình đã thay đổi. Đề xuất nhớ open/close, ghim Apply/dirty, hợp nhất hoặc tách cơ chế Context.
- Phản hồi: A đồng ý panel tùy chọn và nhớ trạng thái, ưu tiên tách Context thành nhóm tự lưu. C được B thuyết phục hợp nhất Context vào draft chung. B chấp nhận panel mở lần đầu nhưng cần nhớ lựa chọn về sau; có thể tách nhóm tự lưu nếu chưa hợp nhất.
- Kết quả: 3/3 đồng ý Apply/dirty phải thấy được và nhớ trạng thái panel. B/C ưu tiên cùng Apply; A ưu tiên nhóm Context tự lưu tách rõ. A/C phản đối modal cảnh báo mỗi lần Send, dùng chỉ báo không chặn. Draft được giữ khi đóng, không có bằng chứng mất dữ liệu.

## Tổng hợp của agent chính

1. Ưu tiên chuyển dock về sidebar để không che hướng dẫn soạn tin.
2. Ghim Apply/trạng thái draft, nhớ trạng thái panel; hướng dài hạn là một lần Apply cho inference + context, cần nối đồng bộ với Cookbook.
3. Tăng chat 14px/hint 12px; thử prose sans và mono bằng cùng transcript trước khi chốt font.

Chỉ lỗi tràn viewport đã được sửa. Các thay đổi do hội đồng đề xuất chưa áp dụng. Không gán điểm chất lượng hoặc giả kết quả user study. Phạm vi bằng chứng: chat trống, panel inference ở 1280×720, dock hai cửa sổ và mã CSS/React; chưa đánh giá hiệu năng inference hoặc hành vi native đầy đủ.


## Đã áp dụng sau khi người dùng đồng ý

- Dock được đưa vào sidebar; vẫn khôi phục được từng công cụ, cả khi sidebar thu gọn.
- Footer Áp dụng/dirty nằm ngoài vùng cuộn, luôn thấy. Panel nhớ mở/đóng; draft giữ khi ẩn, nút mở và composer báo chưa áp dụng.
- Chat/system prompt 14px, hint quan trọng 12px; giữ monospace vì đổi font còn bất đồng.
- Context nhập như draft, cùng nút Áp dụng với inference; SQLite lưu hai setting nguyên tử, run/snapshot đọc cùng query dưới một DB lock. Cookbook dùng context đã lưu; draft context khác được giữ khi Cookbook đổi giá trị và có thông báo.
- Frontend build đạt. Native build và giới hạn runtime ghi trong nhật ký Notion theo kết quả thực tế.
- Đọc render tại kích thước native tối thiểu 1000×700: dock ở sidebar (x6..217), composer x224..720, footer panel bottom700, document height700. Ảnh artifacts/ui/council-applied.png. Không thêm test suite hoặc chạy model thật.
