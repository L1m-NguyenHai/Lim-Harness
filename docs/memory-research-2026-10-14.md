# Nghiên cứu memory cho Lim Harness

Ngày: 2026-10-14, Asia/Bangkok. Phạm vi: đọc mã và đặc tả, so sánh với implementation hiện tại; chưa triển khai kiến trúc mới.

## Kết luận và hướng đề xuất

Lim nên dùng **Obsidian Markdown làm nguồn dữ liệu memory dài hạn**, học cách gắn recall vào runtime từ Odysseus và học cách quản lý fact có nguồn, thời gian, lịch sử từ Obsidian Memory for AI. Đây là đề xuất để triển khai từng bước, chưa phải quyết định đã chốt hoặc cam kết tương thích toàn bộ protocol v4.1.

Một luồng đầu tiên nên hoàn chỉnh: **người dùng yêu cầu nhớ → xem bản ghi và nguồn → xác nhận → ghi Markdown → mở phiên mới → truy hồi đúng nội dung và trích nguồn → sửa fact mà vẫn giữ lịch sử**. Chất lượng của luồng này có giá trị hơn việc thêm embedding ngay.

Ba khái niệm cần tách:

- **Transcript:** cuộc hội thoại nguyên bản, trạng thái run và tool result, tiếp tục lưu trong SQLite của app.
- **Knowledge retrieval:** tìm đoạn trong ghi chú người dùng; một đoạn tìm thấy chưa tự động trở thành fact đã xác nhận.
- **Durable memory:** thông tin được chọn để nhớ lâu dài, có nguồn, phạm vi, thời gian hiệu lực và cách sửa/rút lại.

Tóm tắt phiên là dạng context làm việc hoặc event, không mặc nhiên là sự thật. Câu do model suy luận phải giữ dấu `inferred`; không đổi thành phát biểu đã được người dùng xác nhận.

## Nguồn và phiên bản khảo sát

- Odysseus trong `odysseus/`, commit `a8c147b238db01dbd00b57774ed8b453f1e43971`, remote [odysseus-dev/odysseus](https://github.com/odysseus-dev/odysseus). Nghiên cứu mẫu local, không mặc định đây là HEAD hiện tại của upstream.
- [Obsidian Memory for AI](https://github.com/jrcruciani/obsidian-memory-for-ai), cây mã khảo sát `f87548aad314ab385d4784a618297e363b859238`; README xác định v4.1 là stable hiện tại. Đọc SPEC-v4.1, nền SPEC-v4 và mã v4.1 qua GitHub connector.
- Lim hiện tại: `src-tauri/src/vault.rs`, `runtime.rs`, `storage.rs`, `docs/obsidian-memory.md`; đối chiếu trạng thái Notion.

Không sao chép implementation từ hai nguồn. Odysseus có AGPL-3.0 trong `odysseus/LICENSE`. Cây repo Obsidian Memory for AI khảo sát không thấy tệp LICENSE/COPYING; không suy ra quyền tái sử dụng mã chỉ vì repo public. Nếu lấy mã/tài liệu nguyên văn sau này phải làm rõ giấy phép; nghiên cứu này đề xuất tự xây từ yêu cầu.

## 1. Odysseus: memory trong một harness đang có runtime

Memory native là các entry trong `memory.json`, quản lý bởi `src/memory.py`; đây không phải adapter Obsidian. Luồng chat tại `src/chat_processor.py:328` tải memory theo owner, chia pinned và extended, đưa phần chọn được vào context và ghi nhận memory thực sự đã dùng.

Recall tại `src/chat_processor.py:164` kết hợp điểm từ khóa kiểu BM25 với vector khi vector store khỏe. Keyword dùng hiện diện token và IDF; không nên gọi đây là BM25 đầy đủ. Recency chỉ phá hòa. Core pinned được chọn trước, các pinned khác cần liên quan tới câu hỏi; extended dùng số slot còn lại. Cách này giúp ưu tiên thông tin nền và có đường hoạt động khi thiếu vector.

`src/memory_provider.py:33` định nghĩa contract provider và `:258` có registry. Contract là gợi ý tốt cho ranh giới module của Lim, nhưng cần theo dấu consumer: luồng chat đang gọi MemoryManager/ChatProcessor trực tiếp, không nên coi registry là bằng chứng mọi memory backend đã có thể hoán đổi xuyên suốt.

### Ghi và lifecycle đã theo dấu trong mẫu

| Luồng | Bằng chứng | Điều cần rút ra |
| --- | --- | --- |
| Explicit add/edit/delete | `src/ai_interaction.py:380`, add `:434–450` | Add lưu JSON trước rồi cập nhật vector; index không phải canonical |
| Đọc lỗi khi sửa | `src/memory.py:163`, `:177`, save `:255` | Đọc để hiển thị có thể degrade; đọc để sửa phải báo lỗi, tránh ghi kho rỗng đè dữ liệu |
| Auto extraction | `services/memory/memory_extractor.py:315`, `:335` | Lấy 6 messages gần nhất, có LLM/regex fallback; đây là writer tự động, không phải đề xuất được người dùng duyệt |
| Lịch background | `routes/chat_helpers.py:1716–1720`, `:1641` | Điều kiện là history length từ 4 và chia hết cho 4; bỏ incognito/compare, tôn trọng toggle, đợi session idle |
| Dedup/consolidation | `memory_extractor.py:455–498`, `:137`, `:527` | Cosine 0.72 và exact/fuzzy; audit sau 5 memory mới, có giới hạn xóa; chưa thay thế lịch sử fact |
| Đổi embedding model | `src/embedding_lanes.py:74`, `:136` | Fingerprint model/URL/dimension và lane riêng giúp biết khi nào phải re-embed |

Tại extractor, vector được thêm trước khi JSON được save (`memory_extractor.py:506–518`), khác explicit add. Suy luận từ thứ tự ghi: crash/lỗi save có thể để vector stale. Startup chỉ rebuild khi vector count bằng 0 (`src/app_initializer.py:96`), chưa thấy reconciliation theo content hash trong luồng đã khảo sát. Lim cần một boundary ghi duy nhất và invalidation sau thành công.

`MemoryManager.save()` dùng tệp `.tmp` rồi replace, nhưng chưa thấy khóa/version check trong manager. Atomic replace một file không tránh lost update của hai lượt read-modify-write. Audit có thời gian đợi model rồi merge cũng cần phát hiện thay đổi giữa lúc đọc và ghi. Scheduler đợi session hiện tại và vẫn chạy khi hết thời gian chờ (`chat_helpers.py:1659–1671`); không thể coi đây là khóa backend toàn cục.

Document RAG nằm riêng ở `src/personal_docs.py:102`; skip policy dùng `src/index_walk.py:19`, confinement ở `src/path_confinement.py:160`. Tệp `src/tools/vault.py` là Bitwarden vault, không phải Obsidian. Không thấy luồng đã khảo sát parse wikilink/backlink thành graph memory.

Tests đã tìm và đọc phạm vi, chưa chạy: `test_memory_store_unreadable_no_wipe.py`, `test_memory_provider.py`, `test_memory_extractor_vector_cross_tenant.py`, `test_chat_processor_pinned_memory.py`, `test_embedding_lanes_memory.py`, `test_personal_dir_symlink_escape.py` dưới `odysseus/tests/`. Chúng là checklist hữu ích cho unreadable store, owner filtering, pinned budget, embedding migration và confinement.

Điểm nên học:

- Tách core memory khỏi recall theo câu hỏi; core chỉ gồm một ít thông tin nền cần thiết.
- Đánh dấu memory là dữ liệu không đáng tin cậy về mặt instruction; nội dung nhớ không cấp quyền chạy tool.
- Theo dõi đúng bản ghi đã đưa vào context, thay vì chỉ ghi các kết quả search ban đầu.
- Có fallback keyword khi embedding lỗi, và tách memory cá nhân khỏi RAG tài liệu.

Điểm không nên mang nguyên sang Lim:

- JSON canonical làm người dùng phải duy trì thêm một kho khác ngoài Obsidian.
- Giới hạn số entry đơn thuần chưa đủ cho model context nhỏ: entry dài/ngắn khác nhau; core có thể chiếm hết slot recall.
- Vector top-k lấy toàn cục rồi mới lọc tập memory hợp lệ có thể làm mất kết quả của owner/phạm vi cần tìm. Lim nên lọc phạm vi trước xếp hạng.
- Không cần mang runtime Python, vector dependency và toàn bộ cơ chế tự động ghi của mẫu vào desktop Rust để có memory v1.

## 2. Obsidian Memory for AI: protocol dữ liệu và vòng đời ghi

[SPEC v4.1](https://github.com/jrcruciani/obsidian-memory-for-ai/blob/f87548aad314ab385d4784a618297e363b859238/SPEC-v4.1.md) tổ chức fact, event, narrative và artifacts sinh lại được. Nó bổ sung lịch sử thời gian, nguồn bằng chứng, bootstrap ngắn, alias và kiểm tra memory cũ lên nền transaction/review của v4. Đây là protocol và toolkit portable, chưa phải runtime harness của Lim.

### Những cơ chế có thể áp dụng

| Cơ chế | Bằng chứng trong repo v4.1 | Ý nghĩa cho Lim |
| --- | --- | --- |
| Một fact hiện tại mỗi entity/predicate, bản cũ có lịch sử | `tools/memory_model.py:73`, `tools/transact.py:153` | Sửa thông tin mà vẫn trả lời được “trước đây là gì” |
| Nguồn và loại khẳng định | `tools/query_impl.py:42`, `tools/memory_model.py:104` | Phân biệt stated/inferred/observed; trả được lý do đã nhớ |
| Core context trong budget | `tools/bootstrap.py:107` | Nạp phần nhỏ theo thứ tự cố định; giữ nguyên item, không cắt giữa record |
| Alias và Unicode normalization | `tools/memory_model.py:149`, `tools/rebuild_indexes.py:204` | Tra tên không phân biệt hoa/thường và dấu; kiểm tra alias trùng |
| Proposal gắn hash của nội dung được duyệt | `tools/propose.py:67`, `:155`, `tools/review.py:140` | Không áp dụng một nội dung đã thay đổi sau xác nhận |
| Idempotency và precondition | `tools/transact.py:85`, `:354`, `:372` | Retry không tạo bản ghi trùng; từ chối ghi trên bản đã bị sửa |
| Staging, preimage và recovery | `tools/transact.py:249`, `:276`, `:294` | Có đường khôi phục khi một đợt ghi nhiều file bị gián đoạn |
| Consolidation chỉ chẩn đoán | `tools/consolidate.py:21`, `tools/propose.py:170` | Không để model tự sửa mâu thuẫn bằng một fact đoán mới |

Các đường `tools/` trong bảng nằm dưới [examples/v4.1-minimal-vault](https://github.com/jrcruciani/obsidian-memory-for-ai/tree/f87548aad314ab385d4784a618297e363b859238/examples/v4.1-minimal-vault).

### Những giới hạn cần đọc từ mã thay vì tên tính năng

1. **Index không phải bộ máy tăng tốc truy hồi:** `rebuild_indexes.py:246–251` kiểm tra rendering index rồi gọi scan canonical ở v4.1. Graph cũng xây lại từ dữ liệu gốc. Cơ chế này bảo đảm kết quả không đổi khi index mất/cũ, nhưng chưa giải quyết chi phí đọc vault lớn.
2. **Ghi nhiều file có thể phục hồi, chưa có isolation:** `transact.py:312–326` xuất từng file, ghi journal sau cùng. Một reader khác có thể thấy trạng thái giữa đợt ghi. Mã khảo sát không có `fsync` hoặc khóa liên tiến trình ở publish; không nên quảng bá như transaction database chịu mất điện. `expected_revision` đọc Git HEAD, không thay thế hash của file hoặc tự tạo Git commit.
3. **Trust/role là chính sách hợp tác:** hash phát hiện nội dung proposal đổi, không xác thực danh tính người sửa vault. Confidence là đánh giá được lưu, không phải xác suất đúng đã hiệu chuẩn.
4. **Current và “có hiệu lực hôm nay” cần phân biệt:** `memory_model.py:88` khi không có `as_of` chỉ loại fact đã kết thúc/retracted; bootstrap còn kiểm tra ngày hiện tại. Lim phải định nghĩa thống nhất cách xử lý fact có `valid_from` trong tương lai.
5. **Unicode cần kiểm tra riêng tiếng Việt:** NFKD loại combining mark nhưng không tự đồng nhất `đ` với `d`. Nếu muốn tìm “do an” ra “đồ án”, cần quy tắc bổ sung và test; không dùng alias folding để đổi nội dung hiển thị.
6. **Rút lại khác xóa:** tombstone loại fact khỏi truy hồi hiện tại nhưng Git, backup, transcript và bản sao có thể còn dữ liệu. Không mặc định bật Git hoặc sync cho vault cá nhân.
7. Protocol nhiều schema và thư mục thích hợp cho vault memory được quản lý; không nên yêu cầu người dùng chuyển toàn bộ vault ghi chú sẵn có sang protocol để dùng Lim.

Repo có [21 query contracts](https://github.com/jrcruciani/obsidian-memory-for-ai/blob/f87548aad314ab385d4784a618297e363b859238/tests/eval/questions.yaml) và [regression tests v4.1](https://github.com/jrcruciani/obsidian-memory-for-ai/blob/f87548aad314ab385d4784a618297e363b859238/tests/test_v41_tools.py) về temporal, lineage, trust, hash, fallback, bootstrap và recovery. Đã đọc mã test; chưa chạy suite. Offline query contracts không chứng minh model trả lời đúng hoặc tự trích xuất memory tốt.

## 3. Lim đang ở đâu

| Đã có trong mã | Khoảng trống |
| --- | --- |
| Chọn root vault, kiểm tra canonical path, bỏ symlink/reparse point | Chưa có read-note theo ID/path và dòng nguồn để mở đúng đoạn |
| Scan `.md` có giới hạn tệp/dung lượng/thời gian | Quét lại mỗi câu hỏi; chưa có index tăng dần hoặc theo dõi sửa từ Obsidian |
| So khớp substring, tên/path có trọng số, tie theo path | Chưa parse frontmatter/alias/wikilink, chunk theo heading, lọc fact cũ |
| Context tối đa 4 hit, trích nguồn JSON, bỏ whole hit khi quá budget | Chưa có bootstrap/core memory; budget memory tối đa 4.500 byte, không phải token |
| Lưu source cùng assistant message trong SQLite | Chưa có proposal, ghi vault, lineage, lịch sử fact hay forget |

Bằng chứng: `vault.rs:9`, `:60`, `:98`, `:159`, `:170`; `runtime.rs:348–377`; `storage.rs:427`. Adapter này là **retrieval ghi chú chỉ đọc**, chưa phải vòng đời memory hoàn chỉnh. Notion chưa có bằng chứng đã kiểm tra với vault thực tế.

## 4. Kiến trúc nhỏ đề xuất cho Lim

Giữ vault ghi chú thông thường đọc được như hiện tại. Khi bật ghi memory, người dùng chọn một thư mục do Lim quản lý, ví dụ `LimMemory/`; không sửa tự động ghi chú ngoài thư mục đó.

```text
Vault người dùng
  Ghi chú hiện có/                 đọc và truy hồi
  LimMemory/
    facts/<entity>/<predicate>.md  fact hiện tại
    history/<entity>/<predicate>/  phiên bản đã thay thế
    events/YYYY-MM-DD/             sự kiện và nguồn hội thoại
    proposals/                     đề xuất chờ xác nhận
    receipts/                      ghi nhận lần áp dụng
    views/bootstrap.md             có thể sinh lại
```

Đây là layout gợi ý riêng của Lim, không tự nhận tương thích v4.1. Chỉ tạo các thư mục khi tính năng tương ứng được triển khai. Nhiều giá trị hợp lệ của một predicate cần contract rõ: v1 có thể lưu một list trong một fact; không tạo nhiều “current” cạnh tranh ngầm.

Phân ranh trách nhiệm:

- **Vault store:** đọc Markdown, parse metadata, tạo/sửa trong phạm vi cho phép, kiểm tra hash và ghi receipt. Rust native giữ kiểm tra quyền; model chỉ đề xuất dữ liệu.
- **Retrieval:** scope → loại retracted/history/future khỏi mặc định → normalize/alias → xếp hạng → lấy đoạn và nguồn. Dùng snapshot/hash của nội dung đã đọc.
- **Context builder:** core nhỏ + recall liên quan + transcript, cùng budget. Không nhét cả bootstrap 6.000 ký tự của repo tham khảo vào context 4K của Lim.
- **Memory workflow:** yêu cầu nhớ → proposal → diff/xác nhận → validate → apply → receipt → làm mới dữ liệu dẫn xuất. Sửa fact dùng supersede; rút lại phải làm mất hiệu lực recall.

Metadata tối thiểu đề xuất: `id`, `type`, `entity`, `predicate`, `value`, `recorded_at`, `source_refs`, `assertion`, `status`; thêm `valid_from`, `valid_until`, `supersedes` khi có thay đổi theo thời gian. `pinned` tùy chọn. Lưu nguồn hội thoại bằng session/message ID ổn định; chỉ chép nội dung cần thiết vào event. Không mặc định lưu toàn bộ chat sang vault.

SQLite tiếp tục là nguồn gốc transcript; nếu thêm cache/index memory thì cache nằm ở app data, tái tạo được từ Markdown và gắn với vault identity + file hash. Không dùng path tuyệt đối làm nguồn trích dẫn portable. Chỉ thêm FTS/index tăng dần khi đo thấy scan hiện tại không đủ; embedding sau đánh giá lexical. Kiểm tra compile options trước khi chọn SQLite FTS5.

Không bắt vault phải có Git. Hash file cần kiểm tra ngay lúc apply; một writer trong Lim trước. Khi bổ sung ghi nhiều file, phải thiết kế journal/recovery rõ và không ghi đè sửa đổi của Obsidian. Git chỉ là lựa chọn backup/audit của người dùng.

Đọc không thành công phải tách khỏi kết quả rỗng. Một proposal không được áp dụng nếu tệp cần cập nhật không đọc được. Injection và extraction là hai tùy chọn độc lập; chế độ không lưu phải chặn extraction, ghi event và ghi proposal. Mọi module memory sau này dùng chung boundary native thay vì tự ghi tệp/vượt chính sách.

## 5. Thứ tự làm tiếp và tiêu chí đánh giá

1. **Đo retrieval hiện tại:** fixture có note tiếng Việt, tên trùng, nội dung dài và note không liên quan. Đo top-k đúng, nguồn đúng, không có hit và truncation; sau đó mới chọn cách nâng search.
2. **Đọc có cấu trúc và context:** frontmatter, heading/chunk, alias, read-note, core có budget; câu hỏi không liên quan không được kéo core/context vô hạn.
3. **Ghi có chủ đích:** nút/yêu cầu “nhớ điều này”, proposal nhìn được trước khi áp dụng, chỉ ghi namespace đã chọn. Không cần agent tool loop để chứng minh vòng đời này; runtime có thể gọi thao tác native trực tiếp.
4. **Sửa và rút lại:** supersede, thời gian hiệu lực, source refs, retry idempotent, stale-hash conflict, khôi phục khi gián đoạn.
5. **Sau khi luồng ổn:** extraction cuối phiên dưới dạng proposal, stale detection; index tăng dần và semantic retrieval khi có dữ liệu đo nhu cầu.

Bộ kiểm tra cần thiết khi triển khai (chưa chạy trong lượt nghiên cứu):

- Mở phiên mới vẫn dùng được memory đã xác nhận, trả đúng nguồn.
- Hỏi hiện tại không lấy giá trị cũ; hỏi theo ngày lấy đúng biên thời gian; fact tương lai và retracted không lọt vào mặc định.
- “Đồ án”/“do an”, alias trùng và tên trùng được xử lý có chủ đích.
- Obsidian sửa tệp sau khi proposal được tạo: Lim từ chối apply bằng hash, giữ cả bản sửa và proposal.
- Retry không nhân đôi; cancel/failure không để fact nửa chừng; recovery không phá chỉnh sửa mới hơn.
- Note chứa chỉ dẫn chạy lệnh không trở thành quyền chạy tool.
- Vault vượt giới hạn, file không đọc được, junction/symlink và note lớn có trạng thái rõ ràng.
- Cache bị xóa/cũ không làm mất memory; nguồn trả lời vẫn khớp nội dung đã đưa vào prompt.

Đánh giá model riêng với retrieval: câu trả lời đúng, dùng đúng nguồn, không khẳng định khi thiếu bằng chứng. Đánh giá extraction riêng: tránh lưu suy luận sai, tránh trùng và phát hiện mâu thuẫn. Chưa đặt ngưỡng hay báo điểm khi chưa có dataset và kết quả chạy.

## Trạng thái kết quả nghiên cứu

Đã đọc mã Lim, mẫu Odysseus và spec/mã/test v4.1; đã đối chiếu nguồn gốc canonical, retrieval, context, write governance và recovery. Không chạy Odysseus, suite upstream, model hoặc vault thực tế; không sửa runtime hay mẫu `odysseus/`. Bước tiếp theo đề xuất là tạo fixture và đo retrieval hiện tại, rồi hoàn thành một luồng nhớ có xác nhận xuyên qua hai phiên.
