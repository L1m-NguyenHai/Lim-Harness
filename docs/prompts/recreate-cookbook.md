# Prompt: tái tạo Cookbook động và đánh giá model cho Lim Harness

Bạn đang làm trong repo Lim Harness (Tauri v2 + Rust + React/TypeScript). Hãy mở rộng Cookbook hiện có để khám phá model động và đánh giá khả năng chạy theo máy, lấy cơ chế Odysseus làm tham khảo. Đọc AGENTS.md trước. Giữ UI console compact, cửa sổ nổi, các tab Cookbook và luồng Ollama download/launch/cancel đang có.

Mục tiêu là danh sách dài được cập nhật online, có tìm/lọc/sắp xếp, hiển thị rõ nguồn metadata và lý do đánh giá; không biến thành một danh sách model hardcode. Viết implementation phù hợp repo hiện tại. Không sửa odysseus/, không paste mã mẫu; nếu kế thừa mã trực tiếp phải ghi nguồn và giữ giấy phép cần thiết (mẫu AGPL-3.0). Tài liệu dưới phân biệt thuật toán thực tế của mẫu với các cải tiến yêu cầu cho Lim.

## 1. Bằng chứng và hiện trạng

Nghiên cứu ngày 14/10/2026, mẫu Odysseus commit a8c147b2. Trang http://localhost:7000/ đã xem được: Cookbook Download có Scan/Download, phần cứng RTX 3060 Laptop 6GB VRAM và bảng FIT/MODEL/VRAM/PARAM/QUANT/CTX/SPEED/SCORE/ENGINE. Dữ liệu phần cứng này là một máy tham khảo, tuyệt đối không hardcode vào Lim.

Nguồn trong mẫu (nếu checkout tham khảo có mặt):
- services/hwfit/hf_discovery.py:18,214,274,343: collection discovery và cache.
- services/hwfit/models.py:7,29,40,64,192,282,300: quant constants, memory và hợp nhất catalog.
- services/hwfit/fit.py:184,259,309,314,334,345,354,431,598: tốc độ, điểm và chọn run mode.
- routes/cookbook_routes.py:3986,4025: Ollama HTML discovery + fallback.
- routes/hwfit_routes.py:217: query filter/refresh/hardware.
- static/js/cookbook-hwfit.js:635,648,948: mở rộng Ollama family thành tag rồi gộp bảng HF.

Lim hiện đã có src-tauri/src/catalog.rs, src/model-catalog.ts, src/Cookbook.tsx, cookbook-api.ts, script refresh-model-catalog.mjs và snapshot 67 tag/20 family. Metadata Ollama manifest/config đã có size/quant/params/context/capabilities, cache và fallback. FIT SCORE hiện chỉ đánh giá bộ nhớ; SPEED hiện lấy từ chat thật. Tái sử dụng phần này, không xây lại download engine hoặc bỏ dữ liệu đo thật.

## 2. Odysseus lấy danh sách như thế nào

### Hugging Face

GET https://huggingface.co/api/collections?owner=<owner>&limit=100&expand=true. Theo Link header rel=next, tối đa20 trang/owner, timeout20s/request. Chỉ lấy item type=model, ID hợp lệ; mlx-community chỉ nhận đúng prefix. Các owner của mẫu: mlx-community, zai-org, deepseek-ai, MiniMaxAI, Qwen, stepfun-ai, google, openai, mistralai, meta-llama, NousResearch, moonshotai, mllama.

Lấy numParameters khi có; nếu thiếu mẫu suy luận từ tên repo. Chuẩn hóa repo/provider/params/quant/use_case/context/MoE-active-params/downloads/likes/description/date. Một số metadata trong mẫu được đoán từ tên, gồm quant/context/format/engine; không coi là thông tin đã xác minh. lastModified/lastUpdated được dùng làm ngày trong mẫu, không nhất thiết là ngày phát hành. Model không xác định được params bị bỏ khỏi dòng HF của mẫu.

Cache JSON trong DATA_DIR/hwfit, TTL24h. Force refresh bỏ qua TTL. HF bỏ qua owner bị lỗi; nếu thu được bất kỳ dòng mới nào thì ghi đè cache HF gộp, nên dòng cũ của owner lỗi có thể mất. Nếu toàn bộ refresh không có dòng mới thì dùng cache cũ. MLX refresh là nhánh riêng có thể báo lỗi. Catalog hợp nhất theo thứ tự: catalog người dùng → bundled HF → dynamic HF → dynamic MLX → bundled MLX, dedup theo name. Bundled HF/MLX của checkout mẫu được chủ ý để rỗng; dữ liệu động nằm ngoài source tree.

Trong checkout này, đọc catalog thường chỉ đọc cache; Rescan/fresh hoặc nhánh đã vẽ từ cache và không có search (ngoài image mode) gửi refresh_catalog=1; route gọi refresh_dynamic_catalogs(force=True). TTL24h là kiểm tra của hàm refresh khi force=False, KHÔNG phải scheduler đảm bảo tự cập nhật mỗi24h. Tách mô tả TTL khỏi thời điểm kích hoạt refresh.

### Ollama

Mẫu không dùng /api/tags để tìm model chưa tải. Backend scrape https://ollama.com/search?sort=popular, timeout8s, lấy link /library/<family>, description và chip size; dừng sau80 family tìm thấy. Sau đó thêm các family fallback chưa xuất hiện (nên tổng có thể vượt80). Cache memory1h. Frontend còn có cache library riêng; refresh phải xử lý cả hai tầng.

Mỗi family được mở rộng thành name:size, thiếu size thì name:latest. Parser mẫu:14b→14B,135m→0.135B,8x7b→56B (ước lượng thô). Đây chưa phải xác nhận tag hoặc params thật. /api/tags và /api/ps local chỉ cho biết model đã tải/đang chạy.

### Yêu cầu cho Lim

- Dynamic family discovery thay trần cố định20 family; phân trang/tải thêm với giới hạn tài nguyên rõ ràng, không hứa duyệt toàn bộ HF Hub.
- Kết hợp Ollama Library online + HF collections + catalog người dùng + snapshot offline. Cache ở app data/SQLite, tách TTL Ollama1h và HF24h; refresh catalog tách khỏi scan phần cứng.
- Dùng metadata manifest/config Ollama có sẵn để xác minh tag, size, quant, params, context. Không mặc định mọi tag là Q4 và không sinh tag không tồn tại rồi cho tải.
- Với HF, bổ sung metadata chính thức khi có; giá trị suy luận phải mang provenance/confidence. Source lỗi không xóa cache tốt; hiển thị cache cũ, thời gian cập nhật và cảnh báo.
- Dedup bằng định danh nguồn+repo/tag+quant, không gộp nhầm HF và Ollama chỉ vì tên giống nhau. Aliases cùng weights có thể nhóm nhưng không làm mất khác biệt quant/tag.
- Timeout/retry có backoff cho429/5xx, concurrency giới hạn, hủy refresh được. Chỉ lấy metadata, không tự tải weights.
- HF/llama.cpp/vLLM/MLX được duyệt/đánh giá nhưng hiện Lim chỉ chạy Ollama. Không bật Launch cho backend chưa triển khai; không truyền HF repo ID trực tiếp vào Ollama pull.

## 3. Công thức đánh giá thực tế của Odysseus

Có HAI đường đánh giá khác nhau. Không áp công thức HF cho dòng Ollama rồi tuyên bố đó là thuật toán hiện có của mẫu.

### A. HF/GGUF: bộ nhớ

P = tổng params tính theo tỷ; A = active params tính theo tỷ nếu MoE có metadata, nếu không A=P; C = context token.

M_GB = P × BPP_memory(quant) + 0.000008 × A × C + 0.5

Bảng quant thông dụng:

| Quant | BPP_memory | BPP_speed | Speed multiplier | Quality delta |
|---|---:|---:|---:|---:|
| F16/BF16 |2.0|2.0|0.60|0|
| FP8 |1.0|1.0|0.85|0|
| Q8_0 |1.05|1.0|0.80|0|
| Q6_K |0.80|0.75|0.95|-1|
| Q5_K_M |0.68|0.625|1.00|-2|
| Q4_K_M/Q4_0 |0.58|0.50|1.15|-5|
| Q3_K_M |0.48|0.375|1.25|-8|
| Q2_K |0.37|0.25|1.35|-12|

BPP_memory gồm overhead quant và KHÁC BPP_speed. Mẫu còn có bảng riêng cho AWQ/GPTQ/FP4/QAT/MLX; không tự đồng nhất các format chỉ vì cùng số bit. Nếu chưa có dữ liệu cho format đó, đánh dấu thiếu metadata thay vì giả độ chính xác.

KV theo active params là heuristic của mẫu, không phải công thức KV cache từ kiến trúc attention. Khi Lim có layer/head/head-dimension/cache dtype thật, ưu tiên estimator theo kiến trúc; giữ profile tham khảo và ghi phương pháp tính.

### B. Chọn context/run mode và FIT của HF

Context khởi đầu = min(model max context, context người dùng chọn), hoặc model context khi không chọn. Giữ quant cố định; nếu không vừa, thử context/2 liên tiếp đến1024. Không tự đổi context chat; bảng phải hiện requestedContext và recommendedContext.

_try_quant_at chọn GPU khi M<=VRAM; nếu không và M<=RAM khả dụng thì cpu_offload hoặc cpu_only. GPU-only vô hiệu budget RAM; prequantized native GPU-only không tự chuyển qua CPU.

Phân loại ngân sách r=M/B:
- RAM/unified: PERFECT khi r<=0.50, GOOD khi r<=0.78, MARGINAL khi r<=1, TOO_TIGHT khi vượt.
- GPU discrete ban đầu: PERFECT khi GPU VRAM>=1.5M, GOOD khi>=1.2M, còn lại MARGINAL nếu vừa.
- cpu_offload không được nâng thành PERFECT.
- Sau đó mẫu nâng MARGINAL→GOOD nếu M<=0.78B, và GOOD→PERFECT nếu M<=0.50B và không offload.
- Không vừa kể cả giảm context: vẫn hiện TOO_TIGHT, score/speed0; fitOnly mới loại khỏi danh sách.

Mẫu có quy tắc backend/multi-GPU riêng: một số GGUF đánh giá theo VRAM một GPU, native prequantized theo VRAM tổng, default multi-GPU dùng BF16. Đây là policy của implementation mẫu; không được diễn giải thành kết luận GGUF/llama.cpp luôn không thể chia trên nhiều GPU. Lim v1 một GPU trước; multi-GPU chỉ đánh giá khi biết backend/topology thực sự hỗ trợ.

### C. Tốc độ HF — ước lượng, không phải benchmark

GPU có bandwidth BW(GB/s):
W_active_GB = A × BPP_speed
TPS_est = BW / W_active_GB × 0.55 × (MoE ?0.8:1)

Offload: f=clamp((M-effectiveVRAM)/M,0,1); mẫu dùng f=0.5 nếu offload nhưng không xác định được tỷ lệ.
BW_effective =1 / (f/55 +(1-f)/BW_gpu)
TPS_est = BW_effective/W_active_GB ×0.55 ×(MoE?0.8:1)

Không biết bandwidth hoặc CPU-only:
TPS_est = K_backend/A × SpeedMultiplier(quant)
K: cuda220,rocm180,metal150,cpu_x86 70,cpu_arm90. CPU-only phải dùng CPU backend dù máy phát hiện CUDA.

Mẫu lookup bảng GPU theo tên, match chuỗi dài trước (ví dụ3060Ti trước3060); RTX3060 được gán360GB/s. Tên laptop, biến thể chip và số GPU có thể làm dự đoán sai; không coi số từ bảng là đo thực tế của máy.

### D. QUALITY/SPEED/FIT/CONTEXT và SCORE tổng

QUALITY base theo P:
P<1:30; <3:45; <7:60; <10:75; <20:82; <40:89; còn lại95.

Cộng family:qwen+2,deepseek+3,llama+2,mistral/mixtral+1,gemma+1.
Cộng architecture:qwen3.6+9,qwen3.5+8,qwen3-next+6,qwen3+4,qwen2.5+2 (chọn nhánh phù hợp đầu tiên, không cộng tất cả).
Cộng QualityDelta quant. Coding đúng usecase+6; coding dùnggeneral/chat-10; reasoning đúngusecase vàP>=13 +5; reasoning chochat-4; multimodal đúngusecase+6. Clamp0..100.

Đây là heuristic về kích thước/family/quant, KHÔNG phải điểm benchmark chất lượng.

SPEED_SCORE=clamp(TPS_est/targetTPS×100,0,100).
Target: general/coding/chat/multimodal/tts/stt40; reasoning25; embedding200.

FIT_SCORE với r=M/B, B<=0 hoặcM>B→0:
r<=0.5 →60 +(r/0.5)×40;
0.5<r<=0.8→100;
0.8<r<=0.9→70;
0.9<r<=1→50.
Lưu ý điểm này ưu tiên mức tận dụng bộ nhớ, không tăng đơn điệu theo lượng headroom; không nhầm với badge FIT.

CONTEXT_SCORE: ctx>=target→100; ctx>=target/2→70; còn lại30.
Target context: general/chat/multimodal4096; coding/reasoning8192; embedding512; tts/stt2048.

SCORE=wQ×QUALITY+wS×SPEED_SCORE+wF×FIT_SCORE+wC×CONTEXT_SCORE, làm tròn1 chữ số:

| Use case |wQ|wS|wF|wC|
|---|---:|---:|---:|---:|
| General |.45|.30|.15|.10|
| Coding |.50|.20|.15|.15|
| Reasoning |.55|.15|.15|.15|
| Chat/TTS/STT |.40|.35|.15|.10|
| Multimodal |.50|.20|.15|.15|
| Embedding |.30|.40|.20|.10|

### E. Đường Ollama đơn giản của mẫu

P suy luận từ chip size; M_GB=P×0.6, quant gắn Q4_K_M.
Nếu có VRAM: M<=0.6VRAM→PERFECT, M<=VRAM→GOOD; vượt thì thử RAM.
RAM: r<=.50→PERFECT, r<=.78→GOOD, r<=1→MARGINAL, vượt→TOO_TIGHT. Không có params/budget thì NO_FIT.
SCORE=min(30+0.3P,60); không biếtP thì25. SPEED=0, CTX=0 vì chưa có thông tin, không có nghĩa đã đo tốc độ0.

Frontend mẫu truyền total_ram_gb vào đường Ollama, trong khi HF dùng available_ram_gb. Không gộp VRAM+RAM thành một pool trên GPU discrete. Đây là khác biệt của mẫu phải ghi rõ khi đối chiếu.

## 4. Cách tái tạo phù hợp với Lim

- Tách discovery/metadata, hardware, estimator và ranking thành các phần rõ ràng; không tính mọi thứ trong component React.
- Giữ catalog native và metadata verified hiện tại, bổ sung adapter HF/discovery Ollama rộng hơn. Metadata real được ưu tiên trên suy luận từ tên/chip.
- Có profile đánh giá Odysseus-reference để giải thích/đối chiếu các công thức ở trên; mặc định Lim dùng metadata thật và budget RAM/VRAM khả dụng với headroom hiện có. Trả evaluationMethod/version/assumptions/confidence, không âm thầm trộn hai profile.
- Dùng byte làm đơn vị nội bộ, chuyển GB/GiB rõ ràng. Công thức mẫu dùng GB gần đúng; đừng so giá trị decimalGB trực tiếp với phần cứng GiB mà không chuyển.
- Tổng weights của MoE dùng P, không dùng A; dự đoán tốc độ có thể dùng A. Nếu thiếu A, ghi rõ fallbackP.
- Score tổng đặt tên RECOMMEND SCORE (heuristic), hiển thị breakdown Q/S/F/C; giữ FIT SCORE hiện có như một chỉ số bộ nhớ riêng, không đổi nghĩa nhãn âm thầm.
- Tách estimatedTPS và observedTPS. Số đo thật phải khớp model/context/backend và có ngày đo; nhãn SPEED hiện tại không được thay bằng số dự đoán giả như thể đã benchmark. Khi chưa có số đo hiện —; ước lượng hiện riêng với dấu ~.
- Dữ liệu thiếu để null/—/UNKNOWN và giảm confidence; không biến unknown thành0 hoặc PERFECT. Nếu dùng fallback để tính thì ghi rõ giá trị/nguồn giả định.
- Engine/format/quant không đồng nghĩa. Chỉ bật Download/Launch cho adapter hỗ trợ; HFbrowse-only có linksource và trạng thái chưa hỗ trợ chạy.
- Scan lấy hardware local thực, cache và rescan, có Auto/RAM/GPU; manual hardware chỉ để mô phỏng và luôn gắn nhãn. TổngVRAM không tự suy ra speed nhân theo sốGPU.
- UI bảng dài: FIT, MODEL, EST.MEMORY, PARAM, QUANT, REQUESTED/RECOMMENDED CTX, SPEED thật/ước lượng, RECOMMEND SCORE, ENGINE; filter usecase/capability/engine/official/fitOnly, search model+description, click sort hai chiều. Unknown đứng cuối; sortFIT dùng thứ tựcategory chứ khôngsortstring.
- Official filter chỉ true khi có chứng cứ owner/source đã xác minh; membershipcollection hoặc lượngdownload không tự chứng minhofficial.
- Pagination/virtualization để bảng lớn vẫn dùng được; refresh không làm mất query/filter/scroll/selection hoặc hủy download đang chạy.

## 5. Kết quả phải bàn giao

Bắt đầu bằng phân tích phần hiện có và thiết kế thay đổi nhỏ nhất. Đưa ra quyết định rõ về profile/scoring label và engine browse-only, rồi triển khai luồng discovery→cache→normalize→fit/rank→UI→Ollama download/launch có thể dùng. Không mở rộng runtime HF/vLLM/MLX trong đợt này.

Giữ nguyên mẫu Odysseus. Không tự tải model, đổi provider, push Git hoặc công bố dữ liệu. Chạy build phù hợp; báo đúng kết quả thực hiện. Theo AGENTS.md, thư ký Luna lưu tiến độ vào Notion và phân biệt đã triển khai/đã xác nhận/đề xuất. Chỉ thêm/chạy tests khi người dùng yêu cầu theo quy tắc phiên; các tình huống dưới là tiêu chí nghiệm thu chưa thực hiện.

Tiêu chí nghiệm thu: metadata mới xuất hiện saurefresh mà không sửa snapshotsource; cacheoffline/sourcepartial giữ danh sách; không cótrần20families; phânbiệtparamsM/B/MoEtotalactive; contexttăng kéo memorytăng; GPU-only không mượnRAM; thiếuhardware khôngPERFECT; SPEEDước lượngkhônggiảmấtmeasured; engineunsupported khôngLaunch; parsingHTMLđổi lỗi cófallback; unknown/aliases/malformedrows khôngcrash; UI vẫn giữfloating/downloadjobs.

Fixture đối chiếu tham khảo, không phải benchmark: dense7B Q4_K_M ctx4096 → memory4.789376GB; GPU6GB theo đườngHF cuối cùng GOOD; bandwidth360GB/s →TPS_est56.57. Cùng7B đườngOllama mẫu →memory4.2GB,GOOD,score32.1,khôngcótốcđộđo. Nếu Lim dùng metadata/headroom/profilekhác phải giải thích vì sao kết quả khác.

Nguồn API hiện hành cần kiểm tra khi triển khai: https://huggingface.co/docs/hub/api (OpenAPI được liên kết trong trang), https://ollama.com/search?sort=popular và metadata/manifest adapter sẵn có của Lim. ScrapeHTML là bề mặt có thể đổi, không coi là API listing ổn định.
