# BlackRouter — Kế hoạch nâng cấp UI toàn diện

> **Trạng thái:** Proposal  
> **Phạm vi:** Control Panel tại `/setup` và trang OAuth callback  
> **Mục tiêu:** Nâng Setup UI hiện tại thành một control panel hiện đại, rõ ràng, dễ vận hành và có chất lượng đủ tốt để phát hành như giao diện chính thức của BlackRouter.  
> **Nguyên tắc:** Không thay đổi hành vi proxy/routing đang ổn định; ưu tiên tương thích ngược với control-plane API hiện có.

---

## 1. Bối cảnh hiện tại

Frontend hiện tại là static UI được nhúng trực tiếp trong `blackrouter-api`:

- `crates/blackrouter-api/static/setup.html`
- `crates/blackrouter-api/static/setup.css`
- `crates/blackrouter-api/static/setup.js`
- `crates/blackrouter-api/static/callback.html`

Rust phục vụ các asset bằng `include_str!`, không có frontend build pipeline hoặc Node.js dependency.

Các chức năng UI đã có:

- Xem runtime và database.
- Quản lý API key, tenant, quota và allowlist.
- Cấu hình Telegram.
- CRUD provider, OAuth, test connection và fetch models.
- Xem upstream limits, RTK usage và cost guard.
- CRUD combo và sắp xếp fallback models.
- CRUD model alias.
- Cấu hình proactive health probe.

### 1.1 Vấn đề UX chính

1. UI hiện giống một trang setup/form hơn là control panel vận hành.
2. Không có dashboard tổng quan để biết hệ thống có sẵn sàng hay không.
3. Runtime và Database được đặt ngang hàng với các workflow thường xuyên.
4. Provider/API key/combo form luôn chiếm nhiều diện tích.
5. Trạng thái loading, success, error và confirmation chưa nhất quán.
6. Global refresh phụ thuộc `Promise.all`; một endpoint lỗi có thể ảnh hưởng toàn bộ UI.
7. Mobile navigation sử dụng hàng tab cuộn ngang và khó mở rộng.
8. Chưa có dark mode, URL-based navigation hoặc lưu trạng thái panel.
9. `setup.js` đang gộp API client, state, render và event handling trong một file lớn.
10. Nhiều vùng render bằng `innerHTML`; cần quy ước an toàn rõ ràng hơn.
11. Các thao tác như rotate key, delete alias/combo/provider chưa dùng dialog thống nhất.
12. Provider health, cost, quotas và cảnh báo đang nằm rải rác ở nhiều màn hình.

---

## 2. Mục tiêu sản phẩm

UI mới phải giúp người dùng trả lời bốn câu hỏi trong vòng 10 giây:

1. BlackRouter hiện có nhận request được không?
2. Provider hoặc dependency nào đang gặp vấn đề?
3. Usage, quota và cost có đang tiến gần giới hạn không?
4. Hành động cần làm tiếp theo là gì?

### 2.1 Mục tiêu cụ thể

- Biến `/setup` thành **BlackRouter Control Panel**.
- Tạo dashboard tổng quan có tính hành động, không chỉ hiển thị số liệu.
- Giảm số bước để kết nối provider và tạo routing combo.
- Làm rõ các trạng thái healthy, degraded, disabled, stale và error.
- Chuẩn hóa component, thông báo và hành vi interaction.
- Responsive tốt từ 360px đến desktop rộng.
- Hỗ trợ light/dark mode.
- Keyboard accessible và đạt WCAG AA cho nội dung chính.
- Giữ frontend nhẹ, self-hosted, không tải runtime asset từ CDN.
- Không làm gián đoạn OAuth hoặc control-plane API hiện có.

### 2.2 Ngoài phạm vi

- Không xây thêm user account/login UI trong đợt này.
- Không thay đổi thuật toán routing, quota hoặc provider health.
- Không xây một observability platform thay thế Grafana.
- Không thêm billing/payment flow.
- Không xây chat playground nếu chưa có yêu cầu sản phẩm riêng.
- Không migrate database chỉ để phục vụ visual polish.

---

## 3. Định hướng thiết kế

### 3.1 Phong cách

**Modern infrastructure control panel** — gọn, chuyên nghiệp, information-dense nhưng không rối.

Tham chiếu tinh thần thiết kế:

- Linear: phân cấp rõ, thao tác nhanh.
- Vercel/Railway: dashboard kỹ thuật sạch và hiện đại.
- Grafana: status và số liệu vận hành dễ scan.
- Không sao chép trực tiếp giao diện hoặc branding của sản phẩm khác.

### 3.2 Visual identity đề xuất

- Base color: neutral slate/charcoal.
- Brand accent: emerald/green, giữ liên hệ với palette hiện tại.
- Info: blue.
- Warning: amber.
- Danger: red.
- Border và elevation nhẹ; tránh lạm dụng glassmorphism.
- Radius 8–12px; card hierarchy dựa trên border, spacing và surface.
- Motion nhanh, tinh tế, ưu tiên clarity hơn hiệu ứng trình diễn.
- Typography ưu tiên system font, không phụ thuộc Google Fonts/CDN.
- Dùng SVG icon nội bộ hoặc icon sprite nhỏ; không dùng emoji làm icon chính.

### 3.3 Theme

Hỗ trợ ba chế độ:

- `System`
- `Light`
- `Dark`

Theme được lưu trong `localStorage`, đồng thời cập nhật `color-scheme` để browser control hiển thị đúng.

### 3.4 Motion

- Panel transition: 120–180ms.
- Modal/drawer: 160–220ms.
- Toast: 180ms.
- Skeleton shimmer nhẹ hoặc pulse.
- Tôn trọng `prefers-reduced-motion`.
- Không animation số liệu liên tục gây mất tập trung.

---

## 4. Information Architecture

### 4.1 Navigation mới

```text
Overview

Routing
├── Providers
├── Combos
└── Aliases

Access
└── API Keys

Operations
├── Limits & Cost
└── Settings
    ├── Runtime
    ├── Database
    ├── Health Probe
    └── Telegram
```

### 4.2 URL navigation

Dùng hash routing, không yêu cầu backend route mới:

```text
/setup#overview
/setup#providers
/setup#combos
/setup#aliases
/setup#api-keys
/setup#limits
/setup#settings
```

Yêu cầu:

- Reload giữ đúng panel.
- Back/forward của trình duyệt hoạt động.
- Link có thể chia sẻ.
- Hash không hợp lệ fallback về `#overview`.
- URL query OAuth cũ vẫn được xử lý tương thích.

### 4.3 Desktop layout

```text
┌──────────────┬─────────────────────────────────────────────┐
│ Brand        │ Page title             Health  Refresh     │
│              ├─────────────────────────────────────────────┤
│ Navigation   │                                             │
│              │ Main panel                                  │
│              │                                             │
│              │                                             │
│ Theme/Ver.   │                                             │
└──────────────┴─────────────────────────────────────────────┘
```

- Sidebar cố định 232–256px.
- Main content có max-width hợp lý cho form, nhưng dashboard có thể mở rộng.
- Header sticky trong main content.
- Sidebar có compact/collapse mode nếu cần sau MVP.

### 4.4 Mobile layout

- Sidebar chuyển thành navigation drawer.
- Top bar chứa menu, page title, health status và refresh.
- Form drawer/modal chuyển thành full-screen sheet.
- Data table chuyển thành stacked cards hoặc horizontal scroll có chủ đích.
- Primary action có thể sticky ở bottom trong form dài.

---

## 5. Design System

### 5.1 Design tokens

Chuẩn hóa bằng CSS custom properties:

```text
Color
├── background
├── surface-1 / surface-2 / surface-3
├── border / border-strong
├── text / text-muted / text-subtle
├── primary / primary-hover / primary-soft
├── success / success-soft
├── warning / warning-soft
├── danger / danger-soft
└── info / info-soft

Spacing
├── 4, 8, 12, 16, 20, 24, 32, 40, 48

Radius
├── sm: 6
├── md: 8
├── lg: 12
└── pill: 999

Typography
├── xs: 11–12
├── sm: 13
├── body: 14
├── lg: 16
├── h3: 18
├── h2: 22
└── h1: 28–32

Elevation
├── dropdown
├── modal
└── toast
```

### 5.2 Core components

Các component cần được chuẩn hóa trước khi làm page:

- Button: primary, secondary, ghost, danger, icon.
- Input, textarea, select.
- Checkbox, switch.
- Badge/status pill.
- Card và metric card.
- Data row/data card.
- Tabs nội bộ.
- Dropdown/action menu.
- Modal.
- Drawer/sheet.
- Confirmation dialog.
- Toast.
- Notice/alert.
- Tooltip.
- Progress bar.
- Skeleton.
- Empty state.
- Search/filter control.
- Copyable value.
- Secret value reveal/copy.
- Code block.
- Inline field validation.

### 5.3 Component states bắt buộc

Mọi interactive component cần có:

- Default.
- Hover.
- Active.
- Focus-visible.
- Disabled.
- Loading.
- Error nếu phù hợp.

---

## 6. Chi tiết từng màn hình

## 6.1 Overview Dashboard

Đây là trang mặc định của `/setup`.

### Khối System Health

Hiển thị:

- Gateway: Online / Degraded / Offline.
- Readiness: Ready / Not Ready.
- Uptime.
- Version.
- Database: Healthy / Error.
- Redis/shared state: Connected / Disabled / Error nếu endpoint hiện có cung cấp.

### Nguồn dữ liệu và API contract

Dashboard dùng các endpoint hiện có. Trước khi code, cần audit response shape để xác nhận field available:

| Endpoint | Dùng cho | Cần kiểm tra |
|---|---|---|
| `/health` | Gateway status | `status`, `database` field |
| `/readyz` | Readiness | Dependency checklist shape |
| `/version` | Version | `version` field |
| `/api/runtime/status` | Runtime config | `config`, `storage`, `uptime_seconds` |
| `/api/setup/providers` | Provider list | `data[]` với `is_active`, `auth_type`, `data.models` |
| `/api/provider-health` | Health probe | Response shape, last probe timestamp, failure reason |
| `/v1/models` | Model list | `data[]` với `id`, `owned_by` |
| `/api/provider-limits` | Usage & cost aggregate | `metrics`, `cost_guard`, `data[]` per-provider |
| `/api/rtk/metrics` | RTK rate-limit | `requests_remaining`, `tokens_remaining`, `concurrent_remaining` |
| `/api/usage/daily` | Daily breakdown | Date format, field names |
| `/api/doctor` | Health check | Response shape |

**Quyết định chốt:** Dashboard MVP dùng `/api/provider-limits` làm nguồn chính cho usage & cost (đã có `metrics` + `cost_guard` aggregate). Không cần endpoint dashboard riêng trong release đầu. Chỉ thêm endpoint mới nếu số request hoặc contract trở thành vấn đề thực tế.

Nếu endpoint trả thiếu field cho UI, ghi chú vào bảng trên và quyết định: dùng giá trị mặc định trong UI hoặc bỏ widget đó khỏi MVP.

Hiển thị:

- Tổng số connection.
- Active/disabled.
- Healthy/degraded/unhealthy.
- Provider chưa có model.
- Provider cần attention.

Mỗi row gồm:

- Provider icon/name.
- Account/email/name.
- Health status.
- Model count.
- Last probe nếu dữ liệu có.
- Quick action: test hoặc open provider detail.

Nguồn dữ liệu:

- `/api/setup/providers`
- `/api/provider-health`
- `/v1/models`

### Khối Usage & Cost

Hiển thị:

- Requests hôm nay.
- Prompt tokens.
- Completion tokens.
- Cost hôm nay.
- Cost tháng hiện tại.
- Daily/monthly budget utilization.

### Khối Provider Health

Nguồn dữ liệu:

- `/api/setup/providers`
- `/api/provider-health`
- `/v1/models`

**Lưu ý:** Cần audit `/api/provider-health` response để xác nhận có field `last_probe_at`, `failure_reason`, `consecutive_failures` hay không. Nếu thiếu, UI hiển thị status chung (healthy/degraded/unhealthy) mà không có chi tiết probe.

### Attention Center

Chỉ hiển thị cảnh báo có hành động:

- Không có provider active.
- Gateway API key protection đang tắt.
- Provider unhealthy/degraded.
- Cost vượt hoặc gần budget.
- Database/schema không tương thích.
- Upstream limits stale hoặc gần cạn.
- Redis được cấu hình nhưng không available.

Mỗi alert cần:

- Severity.
- Mô tả ngắn.
- CTA đưa tới đúng màn hình.
- Không hiển thị cảnh báo giả khi endpoint tương ứng lỗi.

### Quick Actions

- Add Provider.
- Create API Key.
- Create Combo.
- Run Doctor.
- Refresh Status.

### Trạng thái dữ liệu

- Dashboard dùng `Promise.allSettled()`.
- Một widget lỗi không làm cả dashboard trắng.
- Widget lỗi hiển thị retry riêng.
- Global health status phân biệt offline thật và partial failure.

---

## 6.2 Providers

### Layout

- Header: title, search, status filter, provider filter, `Add Provider`.
- List là nội dung chính.
- Form create/edit mở trong drawer bên phải trên desktop, full-screen sheet trên mobile.

### Provider row/card

Hiển thị:

- Provider logo/monogram.
- Connection name.
- Email/account.
- Auth type.
- Active/disabled.
- Health status.
- Priority.
- Model count.
- Last test/probe nếu có.

Actions:

- Edit.
- Test.
- View models.
- Fetch models.
- Enable/disable.
- Delete.

Actions phụ đặt trong overflow menu để row không quá chật.

### Add Provider workflow

Ưu tiên wizard nhẹ, không làm form nhiều trang quá phức tạp:

```text
1. Choose provider preset
2. Authentication
3. Connection details
4. Test and save
```

#### Step 1 — Preset

- Grid provider presets.
- Search provider.
- Hiển thị auth type hỗ trợ.
- `Custom provider` là lựa chọn cuối.

#### Step 2 — Authentication

- OAuth button nổi bật nếu hỗ trợ.
- API key/bearer/basic/header theo auth type.
- Mô tả token được lưu ở local database.
- Token field có show/hide và clear.

#### Step 3 — Connection details

- Name.
- Base URL.
- Format.
- Priority.
- Active switch.
- Advanced JSON nằm trong collapsed section.

#### Step 4 — Test and save

- Test connection trước khi save là optional nhưng được khuyến nghị.
- Kết quả test có status, message, HTTP status và retry.
- Có thể save khi test fail nhưng phải xác nhận.

### OAuth UX

Giữ nguyên relay logic hiện có:

- Popup.
- `postMessage`.
- `BroadcastChannel`.
- `localStorage` fallback.
- Status polling.
- Manual code fallback.

UI state chuẩn hóa:

```text
Idle
→ Opening authorization
→ Waiting for provider
→ Exchanging code
→ Token received
→ Testing connection
→ Ready to save
```

- Popup blocked phải có CTA mở link thủ công.
- Timeout không làm mất dữ liệu đã nhập.
- Manual code section chỉ mở khi cần.
- Không hiển thị access token trong toast hoặc log.

### Models view

- Mở trong drawer hoặc expandable section.
- Search model.
- Copy model ID.
- Badge capability nếu metadata có.
- Hiển thị empty state khi chưa fetch.
- CTA `Fetch latest models`.

---

## 6.3 Combos

### List view

Mỗi combo hiển thị fallback chain trực quan:

```text
1  openai/gpt-5
   ↓ fallback
2  anthropic/claude-sonnet
   ↓ fallback
3  google/gemini-pro
```

Thông tin bổ sung:

- Combo name.
- Kind.
- Số models.
- Cảnh báo model/provider không còn tồn tại.
- Edit/Delete actions.

### Combo builder

Mở trong modal/drawer lớn.

Bố cục desktop:

```text
┌─ Combo config ───────┬─ Available models ──────────┐
│ Name                 │ Search                      │
│ Provider filter      │ Provider/model rows         │
│                      │ Add button                  │
│ Ordered fallback     │                             │
└──────────────────────┴─────────────────────────────┘
```

Yêu cầu:

- Search models.
- Filter provider.
- Add từ catalog.
- Manual provider/model input.
- Không cho duplicate.
- Reorder bằng drag-and-drop nếu triển khai ổn định.
- Luôn có keyboard fallback bằng Move Up/Down.
- Remove model.
- Hiển thị index ưu tiên rõ ràng.
- Validation combo name và ít nhất một model.
- Cảnh báo model thuộc provider disabled/unhealthy.

Không thay đổi payload API combo trong đợt UI này.

---

## 6.4 Model Aliases

### List

- Alias → Target.
- Target type: model/combo nếu xác định được.
- Search.
- Edit inline hoặc drawer nhỏ.
- Delete qua confirm dialog.

### Form

- Alias validation.
- Target autocomplete từ model và combo hiện có.
- Vẫn cho phép custom target để giữ tương thích.
- Cảnh báo nếu target không resolve được.

### Empty state

- Giải thích alias dùng để làm gì.
- Ví dụ `fast → openai/gpt-5-mini`.
- CTA tạo alias đầu tiên.

---

## 6.5 API Keys

### List view

Mỗi key hiển thị:

- Name.
- Masked key.
- Tenant.
- Requests/day.
- Tokens/day.
- Cost/month.
- Provider/model restrictions.
- Machine ID nếu có.

Actions:

- Edit Policy.
- Rotate.
- Copy masked ID nếu hữu ích.

Nếu backend chưa có delete/revoke endpoint thì UI không giả lập thao tác đó.

### Create/Edit drawer

Chia form thành nhóm:

1. Identity
   - Name.
   - Tenant ID.
   - Machine ID.
2. Quota
   - Requests/day.
   - Tokens/day.
   - Cost/month.
3. Access policy
   - Provider allowlist.
   - Model allowlist.

Allowlist UX:

- Searchable multi-select từ provider/model hiện có.
- Cho phép custom value để giữ tương thích.
- Hiển thị chip, không yêu cầu chuỗi comma-separated ở trải nghiệm chính.

### Secret reveal flow

Sau create/rotate:

- Modal riêng hiển thị raw API key đúng một lần.
- `Copy API Key` là CTA chính.
- Cảnh báo key sẽ không hiển thị lại.
- Có checkbox hoặc action `I have saved this key` trước khi đóng nếu cần.
- Không lưu raw key vào localStorage.
- Không đưa raw key vào URL hoặc toast.

### Rotate flow

Confirm dialog phải nói rõ:

- Key cũ ngừng hoạt động.
- Tenant và policy được giữ nguyên.
- Ứng dụng đang dùng key cũ cần cập nhật.

---

## 6.6 Limits & Cost

Tách thành ba khu vực rõ ràng:

1. Cost Guard.
2. Gateway RTK.
3. Upstream Provider Limits.

### Summary

- Total requests.
- Prompt tokens.
- Completion tokens.
- Total tracked cost.
- Daily budget.
- Monthly budget.

### Cost Guard

- Enable/disable switch.
- Daily budget.
- Monthly budget.
- Progress bar.
- Threshold:
  - `< 70%`: normal.
  - `70–89%`: warning.
  - `>= 90%`: danger.
  - Exceeded: danger + explicit text.

### Provider limits

Mỗi provider connection hiển thị:

- Health/status.
- Active/disabled.
- Model.
- Snapshot freshness.
- RPM remaining/limit/reset.
- TPM remaining/limit/reset.
- RTK requests/tokens/concurrency remaining.
- Usage requests/tokens/cost.

### Filter và grouping

- Search provider/account/model.
- Filter status.
- Toggle chỉ hiển thị provider cần attention.
- Có thể collapse detail theo provider.

### Freshness

- `Fresh`, `Stale`, `Expired`, `Unknown` phải có text và tooltip.
- Không chỉ dùng màu.
- Hiển thị giờ local và relative age.

---

## 6.7 Settings

Gộp Runtime, Database, Health Probe và Telegram thành một màn hình settings có section rõ ràng.

### Runtime

Read-only information:

- Host.
- Port.
- Data directory.
- Database URL.
- Uptime.
- Version.

Actions:

- Copy endpoint.
- Copy path.
- Mask sensitive URL nếu cần.

### Database

- SQLite path.
- Schema compatibility.
- Table counts.
- Database status.
- Link/CTA chạy doctor.

### Global Security

- Require API key cho `/v1` routes.
- Cảnh báo nếu tắt trong non-local deployment nếu có đủ dữ liệu để xác định.

### Health Probe

- Enabled.
- Interval.
- Timeout.
- Failure threshold.
- Mô tả ngắn tác động của từng trường.

### Telegram

- Enabled.
- Admin IDs.
- Webhook toggle.
- Webhook URL.
- Link code TTL.
- Disabled state cho webhook URL khi webhook off.
- Cảnh báo webhook mode chưa implement đầy đủ nếu backend vẫn chỉ hỗ trợ long polling.

### Config versioning

Endpoint đã sẵn sàng (`GET /api/setup/config/versions`, `POST /api/setup/config/versions/{version}/restore`). Đưa vào release này.

UI bao gồm:

- List settings versions (timestamp, version number).
- Preview diff tối thiểu (hiển thị JSON diff hoặc key changes).
- Restore với confirmation dialog nêu rõ:
  - Version sẽ được áp dụng.
  - Config hot reload sau restore (không cần restart).
  - Không thể undo trực tiếp (phải restore version cũ hơn).
- Thông báo toast sau restore thành công.
- Auto-refresh config form sau restore.

---

## 6.8 OAuth Callback Page

Nâng `callback.html` để đồng bộ visual identity.

States:

- Processing.
- Success.
- Error.
- No code.
- Manual copy fallback.

Yêu cầu:

- Có logo/brand nhỏ.
- Progress indicator khi processing.
- Copy URL button ở manual mode.
- Giữ nguyên security boundary của trusted origins.
- Không render code/token ra UI trừ manual fallback cần thiết.
- Tôn trọng dark/light system theme.

---

## 7. Interaction & Feedback Standards

### 7.1 Toast

Dùng cho:

- Save thành công.
- Provider enabled/disabled.
- Test/fetch hoàn tất.
- Alias/combo tạo hoặc cập nhật.
- Copy thành công.

Không dùng toast cho:

- Raw secret.
- Error cần người dùng xử lý trong form.
- Cảnh báo vận hành lâu dài.

Toast types:

- Success.
- Info.
- Warning.
- Error.

### 7.2 Confirmation Dialog

Bắt buộc cho:

- Delete provider.
- Delete combo.
- Delete alias.
- Rotate API key.
- Restore config version.
- Save provider khi connection test fail, nếu workflow này được hỗ trợ.

Dialog phải nêu:

- Đối tượng bị tác động.
- Hậu quả.
- Action button cụ thể, không dùng `OK` chung chung.

### 7.3 Loading

- Initial page: skeleton dashboard shell.
- Button mutation: spinner + disabled.
- List refresh: giữ dữ liệu cũ, hiển thị subtle loading indicator.
- Không xóa toàn bộ UI khi refresh.
- Chặn double-submit.

### 7.4 Error

Phân loại:

1. Field validation error.
2. Form submission error.
3. Widget/API partial error.
4. Full-page offline state.
5. Control-token/auth error.

Error message nên chứa request ID nếu response cung cấp.

### 7.5 Empty state

Mỗi empty state gồm:

- Tiêu đề.
- Mô tả ngắn.
- CTA chính.
- Ví dụ khi hữu ích.

### 7.6 Control Token Authentication

Khi control plane được bảo vệ bằng `BLACKROUTER_CONTROL_TOKEN`, UI cần xử lý:

**Phát hiện:**
- Gọi một control endpoint (`/api/setup/config`) đầu tiên.
- Nếu response `401`/`403`, hiển thị modal nhập token.

**Flow:**
- Modal nhập control token.
- Token lưu trong `sessionStorage` (không `localStorage`).
- Mọi request `/api/*` gửi header `Authorization: Bearer <token>`.
- Nếu token hết hạn hoặc sai, modal hiện lại.
- Có nút "Forgot token" hướng dẫn kiểm tra env `BLACKROUTER_CONTROL_TOKEN`.

**Bảo mật:**
- Không lưu token vào URL.
- Không render token ra toast/log.
- Không persist token lâu hơn session.
- Token input có type `password` và show/hide toggle.

**Khi control plane không protected:**
- Modal không xuất hiện.
- UI hoạt động bình thường (current behavior).

---

## 8. Frontend Architecture

## 8.1 Quyết định framework

### Khuyến nghị cho đợt nâng cấp này

Tiếp tục dùng **vanilla JavaScript + ES modules**, không migrate ngay sang React/Vue.

Lý do:

- Rust build hiện không phụ thuộc Node.js.
- Static asset được nhúng bằng `include_str!`.
- Control panel có quy mô vừa, không phải application có hàng chục route.
- Giữ binary self-contained và deployment đơn giản.
- Giảm rủi ro ảnh hưởng CI/release.

### Điều kiện cân nhắc framework sau này

- RBAC/multi-user UI.
- Realtime charts phức tạp.
- Plugin UI.
- Nhiều route độc lập.
- Component testing quy mô lớn.
- Cần state synchronization phức tạp.

Nếu cần framework, ưu tiên đánh giá **Preact + Vite** trước React đầy đủ.

## 8.2 Cấu trúc asset đề xuất

```text
crates/blackrouter-api/static/
├── setup.html
├── callback.html
├── css/
│   ├── tokens.css
│   ├── base.css
│   ├── components.css
│   ├── layout.css
│   └── pages.css
└── js/
    ├── app.js
    ├── api.js
    ├── state.js
    ├── router.js
    ├── ui.js
    ├── oauth.js
    └── pages/
        ├── overview.js
        ├── providers.js
        ├── combos.js
        ├── aliases.js
        ├── api-keys.js
        ├── limits.js
        └── settings.js
```

Nếu muốn tránh thêm nhiều Axum route, có thể giữ một CSS/JS bundle thủ công ở iteration đầu. Tuy nhiên cấu trúc module trên được ưu tiên cho maintainability.

## 8.3 Static asset serving

Cập nhật Rust router để phục vụ module/assets nội bộ bằng route rõ ràng hoặc `ServeDir` phù hợp với yêu cầu binary packaging.

Nếu vẫn muốn single-binary hoàn toàn bằng `include_str!`:

- Khai báo route cho từng module.
- Đúng `Content-Type`.
- Có cache header hợp lý cho versioned asset sau này.

Không tải JS/CSS từ CDN.

## 8.4 API client

Tạo lớp API client thống nhất:

- `get`, `post`, `put`, `delete`.
- JSON/text response parsing.
- Timeout với `AbortController`.
- Error normalization.
- Request ID extraction.
- Control-token header nếu UI sau này hỗ trợ nhập token.
- Không retry mutation tự động.
- GET có thể retry một lần cho network error nếu cần.

## 8.5 State model

State tối thiểu:

```text
app
├── route
├── theme
├── loading
├── endpointErrors
├── health
├── readiness
├── version
├── runtime
├── setupConfig
├── providers
├── providerHealth
├── providerLimits
├── providerCatalog
├── models
├── combos
├── aliases
└── apiKeys
```

Không cần đưa toàn bộ state vào một framework store. Dùng module state + render functions hoặc pub/sub nhẹ.

## 8.6 Data refresh strategy

- Initial bootstrap fetch song song bằng `Promise.allSettled()`.
- Mỗi page có refresh riêng.
- Mutation thành công chỉ refresh resource liên quan khi có thể.
- Global refresh vẫn có, nhưng không khóa toàn UI.
- Provider limits có timestamp `last updated`.
- Không auto-refresh quá thường xuyên ở iteration đầu.
- Nếu thêm polling, pause khi tab browser không visible.

## 8.7 Rendering safety

- Ưu tiên `textContent` và DOM creation helpers.
- Nếu dùng template strings, mọi dữ liệu từ API phải escape.
- Không đưa raw HTML từ upstream error vào DOM.
- Không render token/secret trong logs.
- Dùng event delegation có kiểm soát.

---

## 9. Accessibility

### Tiêu chuẩn mục tiêu

WCAG 2.1 AA cho flow chính.

### Checklist

- Semantic heading order.
- Navigation có `aria-label`.
- Active route có `aria-current="page"`.
- Modal dùng `role="dialog"`, `aria-modal="true"`.
- Focus trap trong modal/drawer.
- Focus quay về trigger sau khi đóng.
- `Escape` đóng modal khi an toàn.
- Toast có live region phù hợp.
- Form error liên kết bằng `aria-describedby`.
- Label thật cho mọi input.
- Touch target tối thiểu khoảng 40–44px.
- Focus-visible rõ ràng.
- Không dùng màu làm tín hiệu duy nhất.
- Keyboard hỗ trợ combo reorder.
- Tôn trọng `prefers-reduced-motion`.
- Contrast text/status đạt AA.

---

## 10. Responsive Requirements

### 10.1 Browser Support Matrix

Minimum browser support:

| Browser | Minimum version | Lý do |
|---|---|---|
| Chrome / Edge | 120+ | ES modules, CSS nesting, `color-scheme` |
| Firefox | 120+ | ES modules, `:has()`, custom properties |
| Safari | 17+ | ES modules, `color-scheme`, view transitions |

Yêu cầu kỹ thuật:
- ES modules (native, không bundler).
- CSS custom properties.
- `color-scheme` meta tag.
- `AbortController`.
- `BroadcastChannel`.
- `Promise.allSettled()`.
- `URLSearchParams`.
- Không phụ thuộc polyfill.

Không support:
- IE 11.
- Chrome < 100.
- Safari < 16.

### 10.2 Responsive viewport matrix

Các viewport kiểm thử tối thiểu:

- 360 × 800.
- 390 × 844.
- 768 × 1024.
- 1024 × 768.
- 1280 × 800.
- 1440 × 900.
- 1920 × 1080.

Yêu cầu:

- Không có horizontal page overflow.
- Model IDs dài được truncate hoặc wrap có chủ đích.
- Action row chuyển thành overflow menu trên mobile.
- Modal form dài scroll độc lập.
- Sticky action footer không che input.
- Dashboard cards chuyển 4 → 2 → 1 cột.
- Limits grid chuyển thành stacked sections.
- Navigation drawer đóng sau khi chọn route.

### 10.3 i18n

- UI giữ English cho release đầu.
- Không xây i18n framework trong đợt này.
- Nếu sau này cần đa ngôn ngữ, tách string ra file `js/i18n.js` và load theo `navigator.language`.
- Thứ tự ưu tiên ngôn ngữ khi thêm: English → Vietnamese.
- Không hardcode string trong HTML nếu có thể tách ra dễ dàng, nhưng không bắt buộc trong release đầu.

---

## 11. Security & Privacy

- Không lưu provider token/API key vào localStorage.
- Không render secret vào toast.
- Không đưa secret vào query string.
- Raw gateway key chỉ hiển thị trong secret modal sau create/rotate.
- Copy clipboard cần explicit user action.
- Data JSON advanced section phải cảnh báo có thể chứa secret.
- OAuth relay vẫn giới hạn trusted origins.
- Callback page không post message tới origin tùy ý.
- Error rendering phải escape HTML.
- Không thêm analytics hoặc external telemetry từ browser.
- Không tải font/icon/script từ third-party CDN.

---

## 12. Testing Strategy

## 12.1 Manual UX matrix

Mỗi màn hình phải kiểm thử:

- Loading.
- Success.
- Empty.
- Partial failure.
- Full network failure.
- Validation error.
- Long content.
- 50+ rows.
- Mobile.
- Keyboard only.
- Light/dark mode.

## 12.2 Automated tests

### Rust integration tests

- `/setup` trả HTML đúng content type.
- CSS/JS/module routes trả content type đúng.
- OAuth callback page tồn tại.
- Asset route không yêu cầu control token.

### Browser smoke tests

Khuyến nghị thêm Playwright dưới dạng dev tooling:

1. Mở `/setup` và render Overview.
2. Navigation hash hoạt động.
3. Theme được lưu.
4. Mobile drawer hoạt động.
5. Open/close provider drawer.
6. Create/edit provider với mocked API.
7. Create API key và secret modal.
8. Rotate key confirmation.
9. Create/edit combo và reorder models.
10. Alias create/delete.
11. Partial endpoint failure không làm dashboard crash.
12. OAuth callback relay mock.
13. Keyboard modal focus trap.

Nếu chưa muốn thêm Node trong CI, browser tests có thể triển khai ở phase cuối; nhưng ít nhất cần test static routes và manual checklist.

## 12.3 Performance budget

Mục tiêu ban đầu:

- Không external runtime dependency.
- Tổng CSS + JS uncompressed ở mức hợp lý cho admin UI.
- Initial UI shell render ngay, không chờ toàn bộ API.
- Không có long task đáng kể khi render 100 rows.
- Search/filter debounce 100–200ms nếu cần.
- Không rerender toàn bộ ứng dụng sau mỗi mutation nếu tránh được.

## 12.4 Mock & Fixture Strategy

Browser smoke tests dùng Playwright `page.route()` để intercept `/api/*` mà không cần backend chạy:

```javascript
// Ví dụ: mock provider list
await page.route('**/api/setup/providers', (route) => {
  route.fulfill({ json: { data: [mockProvider] } });
});

// Ví dụ: mock partial failure
await page.route('**/api/provider-health', (route) => {
  route.fulfill({ status: 503, json: { error: 'Service unavailable' } });
});
```

Fixture files đặt tại `tests/fixtures/api/`:

```text
tests/fixtures/api/
├── providers.json
├── provider-health.json
├── provider-limits.json
├── api-keys.json
├── combos.json
├── aliases.json
├── setup-config.json
└── rtk-metrics.json
```

Nguyên tắc:
- Mỗi fixture phản ánh response shape thật (sau API audit ở Milestone 2).
- Test partial failure bằng cách trả lỗi cho 1 endpoint, pass cho其余.
- Không hardcode fixture trong test code; tách file để dễ duy trì.
- OAuth callback test mock `postMessage` thay vì gọi backend thật.

---

## 13. Kế hoạch triển khai

Dù phát hành trong một đợt lớn, implementation nên chia thành milestone nội bộ để kiểm soát rủi ro.

## Milestone 1 — Foundation

**Mục tiêu:** Design system và application shell.

**Điều kiện tiên quyết:** Wireframe low-fidelity cho dashboard, provider list, combo builder và settings (Excalidraw hoặc ASCII layout trong doc). Chốt design direction (color palette, typography, spacing) trước khi code.

Công việc:

- Design tokens light/dark.
- Base typography và spacing.
- Sidebar/topbar/mobile navigation.
- Hash router.
- Button/input/card/badge/empty-state components.
- Toast, modal, drawer, confirm dialog.
- Theme persistence.
- Loading/skeleton base.

Definition of Done:

- Shell responsive.
- Theme hoạt động.
- Navigation deep-link được.
- Core components có đủ state.

## Milestone 2 — Data foundation & Overview

**Mục tiêu:** API client bền vững và dashboard tổng quan.

Công việc:

- **API contract audit**: đọc response thật của tất cả endpoint dashboard dùng, ghi chú field available vs field cần (xem bảng Section 6.1).
- API client và normalized errors.
- `Promise.allSettled()` bootstrap.
- Partial widget errors.
- System/provider/usage/cost cards.
- Attention Center.
- Quick actions.
- Run Doctor UI.
- Control token detection flow (Section 7.6).

Definition of Done:

- Một endpoint lỗi không làm dashboard crash.
- Dashboard trả lời được bốn câu hỏi vận hành chính.
- API contract audit hoàn tất và fixture files tạo xong.

## Milestone 3 — Providers & OAuth

**Mục tiêu:** Hoàn thiện workflow quan trọng nhất.

Công việc:

- Provider list/search/filter.
- Add/edit drawer.
- Preset selection.
- Dynamic auth fields.
- Test/fetch/toggle/delete actions.
- Models viewer.
- OAuth state presentation mới.
- Callback page redesign.

Definition of Done:

- Tất cả OAuth providers hiện có vẫn login được.
- Manual fallback vẫn hoạt động.
- Provider CRUD không mất chức năng.

## Milestone 4 — API Keys

**Mục tiêu:** Quản lý key an toàn và rõ quota.

Công việc:

- API key list.
- Create/edit policy drawer.
- Provider/model multi-select.
- Secret reveal modal.
- Copy action.
- Rotate confirmation.

Definition of Done:

- Raw key không xuất hiện ngoài secret modal.
- Edit/rotate giữ tenant và policy theo backend behavior.

## Milestone 5 — Combos & Aliases

**Mục tiêu:** Routing configuration trực quan.

Công việc:

- Combo chain visualization.
- Search/filter model picker.
- Ordered draft editor.
- Keyboard reorder.
- Validation và unavailable target warning.
- Alias autocomplete/edit/delete.

Definition of Done:

- Combo payload tương thích API cũ.
- Không cho duplicate model trong draft.
- Alias custom target vẫn được hỗ trợ.

## Milestone 6 — Limits, Cost & Settings

**Mục tiêu:** Hoàn chỉnh operations UX.

Công việc:

- Summary cards.
- Cost guard progress.
- Provider limits grouping/filter.
- Freshness indicators.
- Runtime/database cards.
- Health probe và Telegram settings.
- Config version history list.
- Config version restore với confirmation dialog.
- Config diff preview (tối thiểu JSON diff).

Definition of Done:

- Cost/limit trạng thái dễ scan.
- Save settings có loading/success/error rõ ràng.
- Config version restore hoạt động và hot reload confirmed.

## Milestone 7 — Quality Pass

**Mục tiêu:** Polish, accessibility và regression protection.

Công việc:

- Responsive pass.
- Keyboard/focus pass.
- Contrast audit.
- Long content/large dataset pass.
- Reduced-motion pass.
- Browser smoke tests.
- Rust static asset tests.
- Documentation/screenshots.

Definition of Done:

- Không còn P0/P1 UI defect.
- Các flow cũ đều có regression checklist pass.

---

## 14. Ước tính nguồn lực

Ước tính cho một developer đã quen codebase và **có design direction chốt trước**:

| Hạng mục | Ước tính |
|---|---:|
| Wireframe / visual direction | 2–3 ngày |
| Foundation + design system | 3–4 ngày |
| Dashboard + API state + contract audit | 4–5 ngày |
| Providers + OAuth | 4–6 ngày |
| API Keys | 2–3 ngày |
| Combos + Aliases | 3–4 ngày |
| Limits + Settings + config versioning | 4–5 ngày |
| Accessibility + responsive + tests | 3–5 ngày |
| **Tổng** | **25–35 ngày làm việc** |

**Giả định:**
- Developer quen codebase Rust + existing API.
- Design direction (color, typography, layout) chốt trước khi code.
- Không có designer riêng; nếu có, thêm 5–7 ngày cho visual iteration vòng đầu nhưng giảm số vòng polish.
- Nếu không có wireframe trước, thêm 2–3 ngày cho trial-and-error layout.
- Estimate không bao gồm thời gian review/merge.

Nếu chỉ có một developer và vẫn muốn release một lần, nên phát triển trên branch riêng và merge sau khi Milestone 7 hoàn tất.

---

## 15. Release Strategy

### Phương án khuyến nghị

- Implement theo milestone trên branch riêng.
- Không phát hành từng phần ra main nếu shell mới chưa ổn định.
- Trước merge, chạy regression toàn bộ control-plane flows.
- Có thể giữ UI cũ tạm thời dưới feature flag hoặc route nội bộ trong quá trình phát triển, nhưng xóa sau release nếu không còn cần.

### Rollback

Vì backend API không đổi, rollback UI chỉ cần quay lại static assets và route asset tương ứng.

Không nên gắn database migration bắt buộc vào UI release để giữ rollback đơn giản.

---

## 16. Acceptance Criteria tổng thể

UI upgrade được xem là hoàn thành khi:

### Functional

- Tất cả chức năng UI cũ vẫn hoạt động.
- Overview dashboard hoạt động với partial API failure.
- Provider CRUD/test/fetch/toggle/OAuth hoạt động.
- API key create/edit/rotate hoạt động.
- Combo CRUD/reorder hoạt động.
- Alias CRUD hoạt động.
- Limits/cost guard hoạt động.
- Runtime/database/config/Telegram/health probe hoạt động.

### UX

- Người dùng xác định system health và attention items trong tối đa 10 giây.
- Primary action của mỗi page rõ ràng.
- Mutation có loading/success/error feedback.
- Destructive action có confirmation.
- Empty state có CTA.
- Raw secret được xử lý an toàn.

### Responsive

- Không page overflow tại 360px.
- Navigation mobile hoạt động.
- Form và data list dùng được bằng touch.
- Dashboard và limits stack hợp lý.

### Accessibility

- Keyboard điều hướng được toàn bộ flow chính.
- Focus modal/drawer đúng.
- Contrast đạt AA cho nội dung chính.
- Status không phụ thuộc màu duy nhất.
- Reduced motion được tôn trọng.

### Technical

- Không thêm external CDN/runtime dependency.
- Backend control-plane API không bị breaking change.
- Static assets được phục vụ đúng content type.
- OAuth security behavior không bị nới lỏng.
- Rust tests và browser smoke tests liên quan đều pass.

---

## 17. Rủi ro và phương án giảm thiểu

| Rủi ro | Mức độ | Giảm thiểu |
|---|---|---|
| Rewrite lớn gây regression OAuth | Cao | Giữ OAuth logic tách module, test từng provider và manual fallback |
| UI mới phụ thuộc endpoint chưa ổn định | Trung bình | Adapter/normalization layer, widget-level error handling |
| `setup.js` tiếp tục phình to | Cao | Tách ES modules ngay trong milestone foundation |
| Static module routing làm Rust route phức tạp | Thấp | Route rõ ràng hoặc build thành một bundle nội bộ |
| Mobile data-heavy khó dùng | Trung bình | Card view, collapse detail, overflow menu |
| Secret vô tình xuất hiện trong toast/log | Cao | Secret modal riêng, security review checklist |
| Dark mode contrast không đạt | Trung bình | Token audit và automated contrast spot checks |
| Scope polish kéo dài | Trung bình | Freeze design system sau Milestone 1, acceptance criteria rõ ràng |
| Một release quá lớn khó review | Cao | Commit/PR nội bộ theo milestone dù release cùng lúc |
| Control token flow gây friction | Trung bình | sessionStorage + auto-retry, chỉ hiện modal khi 401/403 |
| API field thiếu cho dashboard | Trung bình | API audit ở Milestone 2, fallback default values |
| Browser compatibility (Safari) | Thấp | Support matrix rõ ràng, test trên Safari 17+ |
| Config version restore conflict | Thấp | Hot reload atomic, confirmation dialog rõ ràng |

---

## 18. Quyết định kỹ thuật cần chốt trước khi code

1. Giữ một CSS/JS file hay tách ES modules và nhiều CSS files?
2. Có chấp nhận Playwright/Node.js chỉ cho dev và CI không?
3. Config version history/restore đưa vào release này? → **Đã chốt: Có** (endpoint sẵn sàng).
4. Overview usage dùng endpoint nào? → **Đã chốt: `/api/provider-limits`** cho MVP.
5. Provider health endpoint trả đủ field cho UI không? → **Cần audit ở Milestone 2.**
6. Có cần nhập control token trong UI không? → **Có** (Section 7.6), lưu sessionStorage.
7. Có cần feature flag cho UI mới trong giai đoạn phát triển không? → **Không**, phát triển trên branch riêng.
8. Drag-and-drop combo có bắt buộc? → **Không**, keyboard reorder là đủ cho release đầu.
9. i18n cần không? → **Không** cho release đầu, UI giữ English.
10. Browser support minimum? → **Chrome/Edge 120+, Firefox 120+, Safari 17+** (Section 10.1).

### Khuyến nghị mặc định (đã chốt)

- Tách ES modules.
- Chấp nhận Playwright dưới dạng dev-only tooling.
- Config version restore **đưa vào release này**.
- Dashboard dùng `/api/provider-limits` cho MVP.
- Provider health field audit ở Milestone 2.
- Control token UI: modal + sessionStorage (Section 7.6).
- Keyboard reorder là bắt buộc; drag-and-drop là enhancement.
- Không thêm control-token persistence vào `localStorage`.
- Không i18n framework trong release đầu.
- Phát triển trên branch riêng, không feature flag.

---

## 19. Thứ tự ưu tiên nếu cần cắt scope

Nếu tiến độ buộc phải giảm phạm vi nhưng vẫn phát hành UI mới đồng bộ:

### Không được cắt

- Design system.
- Responsive shell.
- Overview dashboard.
- Provider/OAuth workflow.
- API key secret handling.
- Toast/loading/error/confirmation standards.
- Accessibility cơ bản.

### Có thể lùi sau

- Drag-and-drop combo.
- Config version diff viewer.
- Advanced provider filters.
- Animated charts.
- Sidebar collapse desktop.
- Realtime auto-refresh.
- Rich model capability badges.

---

## 20. Kết quả kỳ vọng

Sau nâng cấp, BlackRouter sẽ có một control panel:

- Đẹp và nhất quán ở cả light/dark mode.
- Thể hiện đúng bản chất của một AI gateway/router chuyên nghiệp.
- Dễ sử dụng cho người mới nhưng vẫn đủ dữ liệu cho vận hành.
- Không làm tăng độ phức tạp deployment một cách không cần thiết.
- Có nền tảng frontend đủ sạch để tiếp tục mở rộng observability, multi-tenancy hoặc plugin UI trong tương lai.
