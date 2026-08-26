# v1.28.31 — Arabic strings (first cut; RTL locale for the G4 readiness pass).
# Simple FTL subset: `key = value`, `#` comment lines, blank lines skipped.
# A key missing here falls back to `en` via i18n.rs `t()` — but the parity
# wall (locale_key_sets_are_identical) requires the exact `en` key set.

## Panel titles
review_title = قائمة المراجعة
recall_title = مفتش الاستدعاء
subjects_title = أصحاب البيانات (DSAR)
security_title = الأمان
sec_released = خرج من الحجر الصحي
sec_deleted = محذوف
audit_title = التدقيق
health_title = الحالة

## Navigation
nav_review = المراجعة
nav_recall = الاستدعاء
nav_subjects = أصحاب البيانات
nav_security = الأمان
nav_audit = التدقيق
nav_health = الحالة

## Shell / top bar
pending = معلّق
flags = علامات
audit_chain = سلسلة التدقيق!
acting_as = تتصرف باسم
loopback = اتصال محلي
sign_out = تسجيل الخروج
connected = متصل
reconnecting = إعادة الاتصال
disconnected = انقطع الاتصال — يُعرض آخر حالة معروفة. إجراءات الكتابة معطّلة.
reverifying = عاد الاتصال — يتم التحقق من سلسلة التدقيق قبل تمكين الكتابة…
detail = تفاصيل
close_drawer = إغلاق اللوحة
nothing_selected = لم يُختر شيء

## Connect
connect_title = الاتصال بـ brain-server
connect_welcome = ذاكرة خاضعة للحوكمة، على عتادك.
backend_url = عنوان الخدمة
token_label = الرمز
url_placeholder = فارغ = مصدر هذه الصفحة (نفس الخادم)
token_placeholder = اختياري (اتصال محلي)
token_access_placeholder = رمز الوصول (JWT)
jwt_pair = زوج JWT (وصول + تحديث) — يتيح التحديث الصامت
refresh_token_label = رمز التحديث
refresh_token_placeholder = من `brain key mint` أو مزود الهوية
connecting = جارٍ الاتصال…
connect_button = اتصال
plaintext_http = http:// غير مشفّر عبر عنوان غير محلي — سيسافر رمز المصادقة دون تشفير. استخدم https:// أو مضيفًا محليًا.
install_hint = تثبيت بسطر واحد:  curl -fsSL … | sh   ثم  brain doctor

## Review
no_pending = لا توجد مقترحات معلّقة.
approve = موافقة
reject = رفض
edit = تعديل
proposal = مقترح
review_help = اختصارات لوحة المفاتيح للمراجعة
review_help_toggle = إظهار/إخفاء مساعدة الاختصارات
review_key_approve = الموافقة على المقترح المحدد
review_key_supersede = الموافقة مع إلغاء المقترح المتعارض
review_key_reject = رفض المقترح المحدد
review_key_edit = إعادة صياغة محتوى المقترح المحدد
review_key_next = المقترح التالي
review_key_prev = المقترح السابق

## Subjects
deletion_certificate = شهادة حذف {0}
chain_verified = السلسلة موثقة
chain_tampered = السلسلة فُسِدت
dsar_preview_title = معاينة أثر طلب البيانات
dsar_preview_sub = انظر بالضبط ماذا سيحذف المحو — لا يُمحى شيء الآن.
dsar_preview_placeholder = صاحب البيانات للمعاينة…
dsar_preview_button = معاينة الأثر
dsar_preview_note = معاينة فقط — لم يُحذف شيء.
dsar_preview_owners = مالكو البيانات
dsar_preview_derived = مشتقات
dsar_preview_export_rows = صفوف التصدير
dsar_preview_tombstones = شواهد قبور سابقة
dsar_preview_ledger_rows = صفوف السجل

dsar_clock_title = سجل طلبات الوصول إلى البيانات
dsar_clock_empty = لا طلبات مفتوحة — النافذة خالية.
dsar_clock_completed = مكتمل
dsar_clock_retained = السجل محفوظ وفق سياسة الاحتفاظ
dsar_clock_deadline = المهلة

## Settings
theme_label = المظهر
locale_label = اللغة
density_label = الكثافة
dark = داكن
light = فاتح
comfortable = مريح
compact = مضغوط

## Privacy posture
privacy_title = الخصوصية
privacy_sends = يرسل العميل:
privacy_sends_1 = طلبات API إلى: [عنوان خدمتك]
privacy_sends_2 = Authorization: Bearer [رمزك]
privacy_stores = يخزن العميل:
privacy_stores_1 = رمز المصادقة (في سلسلة مفاتيح النظام — لا تصل إليه تطبيقات أخرى)
privacy_stores_2 = تفضيلات المظهر/اللغة (غير حساسة)
privacy_not = العميل لا يقوم بـ:
privacy_not_1 = إرسال تحليلات أو قياسات أو تقارير أعطال
privacy_not_2 = الاتصال بأي خادم غير الذي تهيئه
privacy_not_3 = تخزين محتوى الذاكرة محليًا
privacy_not_4 = استخدام حزم SDK خارجية أو شبكات CDN أو موارد خارجية

## Overview
nav_overview = نظرة عامة
overview_title = نظرة عامة
overview_health = الحالة
overview_snapshot = سلامة اللقطات
overview_retention = الاحتفاظ
overview_ump = الخادم + UMP
overview_alerts = التنبيهات
no_alerts = لا تنبيهات — كل هادئ.
open_queue = قائمة مفتوحة
view = عرض
kinds = أنواع
alert_auth_failures = إخفاقات مصادقة
alert_quarantine = أجزاء في الحجر الصحي
alert_stale_sources = مصادر متقادمة
alert_conflicts = تعارضات غير محلولة
alert_decayed = أجزاء مت decayed
alert_near_duplicates = أجزاء شبه مكررة
alert_tombstones = شواهد قبور

## Command palette
palette_recent = الأخيرة
palette_go_to = الانتقال إلى
palette_lookup = بحث
palette_run = تنفيذ
confirm_destructive = اضغط Enter للتأكيد

## Graph
nav_graph = المخطط
graph_title = المخطط

nav_queued = في الانتظار
nav_queued_title = إجراءات صدرت أثناء عدم الاتصال؛ يُعاد تنفيذها عند عودة الاتصال
graph_entity_ph = ابحث عن كيان…
graph_browse = كيان
graph_type = نوع
graph_relations = علاقات
graph_traverse = اجتياز
graph_start = كيان البداية
graph_depth = أقصى عمق
graph_kind = نوع الحافة (اختياري)
graph_at = صالح عند (اختياري)
graph_cross_domain = عبر النطاقات
graph_run = اجتياز
graph_rel = علاقة
graph_out = صادر
graph_in = وارد
graph_no_entity = لا يوجد such كيان
graph_paths = مسارات
graph_rows = صفوف
none = لا شيء

## Create
nav_create = إنشاء
create_title = إنشاء
create_sub = أدوات الكتابة: إدخال ذاكرة، بناء إجراءات، توطيد.

## Ingest
ingest_title = إدخال
ingest_tab_structured = مهيكل
ingest_tab_markdown = Markdown
ingest_tab_memory = دفعة ذاكرة
ingest_content = المحتوى
ingest_kind = نوع الذاكرة
ingest_domain = النطاق
ingest_entities = الكيانات (JSON)
ingest_relations = العلاقات (JSON)
ingest_source_path = مسار المصدر
ingest_replace = استبدال الموجود
ingest_submit = إدخال
ingest_bad_json = يجب أن تكون الكيانات/العلاقات JSON صالحًا
ingest_mem_hint = ذاكرة لكل سطر، مع عناوين ## اختيارية
outcome_created = أُنشئ
outcome_duplicate = مكرر (موجود مسبقًا)

## Procedures
proc_title = الإجراءات
proc_step_title = عنوان الخطوة
proc_step_body = محتوى الخطوة
proc_add_step = إضافة خطوة
proc_create = إنشاء إجراء
proc_steps = الخطوات
proc_is_decision = قاعدة قرار
proc_created = أُنشئ الإجراء ({0} خطوات)
cls_title = تصنيف
cls_text = نص للتصنيف
cls_run = صنِّف
dec_title = تقييم القرار
dec_id = معرف القرار
dec_vars = المتغيرات (JSON)
dec_run = قيّم

## Consolidate
cons_title = توطيد
cons_load = تحميل
cons_apply = الموافقة على الإلغاء والاستبدال
cons_undo = تراجع
cons_empty = لا شيء لتوطيده.
cons_near_dup = شبه مكرر
cons_conflict = تعارض
cons_applied = طُبق {0} استبدالًا
cons_undone = تراجع عن {0}

## Data
nav_data = البيانات
data_title = البيانات
data_sub = الحقوق وقابلية النقل: محو، تصدير، احتفاظ، سجلات.
data_status = جاهز
data_purge = محو
data_export = تصدير
data_exported = وُلد التصدير
data_retention = الاحتفاظ
data_retention_state = الاحتفاظ
data_retention_kind = النوع
data_retention_days = أيام
data_retention_bad_days = يجب أن تكون الأيام رقمًا صحيحًا
data_retention_set = حُدث {0} تجاوزًا
data_decayed = مت decayed
data_next_expiry = التالي في الانتهاء
data_tombstones = شواهد القبور
data_empty = لا شيء للعرض.
data_purge_ids = معرفات الأجزاء (مفصولة بفواصل/مسافات)
data_purge_owner = أو امسح كل ما يخص مالكًا
data_purged = مُسح {0} جزءًا
data_purged_queued = في قائمة الانتظار (غير متصل) — سيُمحى عند عودة الاتصال
data_purge_empty = قدّم معرفات الأجزاء أو مالكًا

## UMP
nav_ump = UMP
ump_title = UMP
ump_sub = عمليات UMP 1.0: القدرات، التذكر، الاستدعاء، التدقيق.
ump_caps = القدرات
ump_remember = تذكّر
ump_recall = استدعِ
ump_audit = تدقيق
ump_bad_json = JSON غير صالح
ump_remembered = تُذكِّر
ump_chain_ok = السلسلة موثقة
ump_chain_bad = السلسلة فُسِدت

## System
nav_system = النظام
sys_title = النظام
sys_sub = لوحة المشغل: النطاقات، اللقطة، المادة 30، المصادر، وحدة التجربة.
sys_domains = النطاقات
sys_snapshot = سلامة اللقطات
sys_art30 = سجل المادة 30
sys_reindex = إعادة الفهرسة
sys_reindexed = أُعيدت فهرسة {0} جزءًا
sys_sources = المصادر والموصلات
sys_console = وحدة التجربة

## Operations
nav_ops = العمليات
ops_title = العمليات
ops_sub = سطح العمل الحيوي: قائمة المعلقات، ساعات SLA، والمخرجات المعلمة بالفلترة.
ops_queue = القائمة الحية
ops_queue_summary = معلق
ops_queued_offline = في الانتظار (غير متصل) — سيُعاد عند الاتصال
ops_gate = صحة البوابة
ops_flagged = معلم ومحجور
ops_flagged_hint = أدخل استعلامًا تجريبيًا وامسح لإظهار المطابقات المعلمة.
ops_flagged_empty = لا مطابقات معلمة.
ops_decayed = مت decayed (فاتت المهلة)
ops_scan = مسح
ops_sourcing = استعلام المصدر
ops_expired = منتهٍ (رُفض تلقائيًا)
alert_queued = قُترح جديد في الانتظار
alert_screen = حقن مُعلَّم
alert_expiring = مقترح يوشك على الانتهاء
sla_critical = حرج
sla_warn = ينتهي قريبًا
sla_remaining = متبقٍ
gate_healthy = سليمة
gate_over_rejecting = رفض مفرط
gate_under_reviewing = مراجعة ناقصة

## Register
nav_register = السجل

## Console
nav_clients = العملاء
console_client_title = لوحة العميل
console_client_sub = نظرة قراءة فقط على العملاء الممنوحين لرمز المدقق. مقيدة بنطاقات JWT.
console_ops_title = لوحة عمليات BPO
console_ops_sub = سجل كل العملاء + الموصلات + حمل المراجعة. قراءة فقط.
console_ops_board = سجل العملاء
console_connectors = الموصلات
console_queue_depth = عمق القائمة
console_empty = لا شيء لهذا الرمز.

## Replay
replay_title = إعادة قرار
replay_audit_link = فتح سجل التدقيق
replay_export = تصدير الأدلة

## Calibrate
cal_title = بوابتك الأخيرة
cal_approve_rate = نسبة الموافقة
cal_latency = وسيط القرار
cal_edit_rate = نسبة التعديل
cal_override_rate = نسبة تجاوز الفلترة
cal_decisions = قرارات
cal_last_200 = آخر 200 قرار
cal_warn_high = نسبة موافقة عالية — راجع آخر قرارات يدويًا

wizard_title = ما يصف فريقك أفضل وصف؟
wizard_hint = اختر وضعًا بدايًا — كل إعداد يظل قابلًا للتعديل لاحقًا.
wizard_apply = استخدم هذا الملف الشخصي
wizard_applied = طُبق — ضبطت الافتراضيات، لم يعاد إدخال شيء
wizard_skip = تخطَّ الآن
wizard_load_failed = تعذر تحميل الملفات الشخصية
health_profile = الملف الشخصي
health_profile_none = لا شيء — افتراضيات الخادم
health_profile_knobs = الإعدادات الفعلية

data_purge_preview = معاينة الأثر
data_purge_preview_note = لم يُحذف شيء بعد — هذا هو النطاق المحتمل.
data_purge_preview_stale = تغير الإدخال منذ هذه المعاينة — أعد تشغيلها قبل المحو.
data_purge_need_preview = شغّل المعاينة أعلاه أولًا — المحو يتطلب أثرًا معروضًا.
data_purge_hint = شغّل معاينة الأثر لتفعيل المحو.
purge_irreversible = هذا لا رجعة فيه.
reindex_irreversible = يعيد بناء مخزن المتجهات — هذا لا رجعة فيه.
quarantine_delete_irreversible = حذف نهائي للجزء المحجور — هذا لا رجعة فيه.
dsar_purge_confirm_title = أمحو صاحب البيانات هذا؟ الأثر أعلاه هو النطاق المؤكد.
dsar_purge_confirm = امحُ الآن
dsar_purge_need_preview = لم يُعرض الأثر لصاحب البيانات الحالي — شغّله أولًا.
replay_title = إعادة الإجراءات التي لا رجعة فيها
replay_sub = انتظرت أثناء عدم الاتصال — لا تُطلق تلقائيًا أبدًا. أعد كل واحدة أمامك، أو تخطَّها.
replay_kind_approve = موافقة
replay_kind_reject = رفض
replay_kind_edit = تعديل
replay_kind_purge = محو
replay_kind_dsar = محو DSAR
replay_queued_ago = في الانتظار
replay_replay = إعادة
replay_skip = تخطٍّ
replay_dismiss = تجاهل الكل
replay_subject_prompt = أعد إدخال صاحب البيانات لمحوهم (النموذج دون اتصال حفظ الهاش فقط):
replay_subject_placeholder = صاحب بيانات / مالك / جهة…
replay_subject_required = أعد إدخال صاحب البيانات أولًا — الهاش أحادي الاتجاه.

back_to_queue = ← عودة إلى قائمة المراجعة
detail_loading = تحميل المقترح…
retry = إعادة المحاولة
detail_not_pending = لا مقترح معلق #{0} (حُسم مسبقًا؟)
digest_label = البصمة
copy_digest = نسخ البصمة

graph_need_start = أدخل اسم كيان للبدء منه
graph_need_kind = أدخل نوع علاقة صالحًا

audit_error = فشل التدقيق: {0}
audit_empty = لا أحداث.
audit_filtered_summary = {0} حدثًا محملًا · {1} بعد الفلترة (هاشات فقط — بلا محتوى خام)
audit_principal_placeholder = الجهة…
audit_filter_principal = فلترة حسب الجهة
audit_filter_kind = فلترة حسب النوع
audit_all_kinds = كل الأنواع
audit_filter_since = فلترة من تاريخ
audit_export = تصدير JSON
audit_rows_exported = صُدّر {0} صف تدقيق
audit_load_more = تحميل المزيد ({0} محمل)

dsar_subject_placeholder = صاحب بيانات / مالك / جهة…
dsar_subject_aria = صاحب بيانات للإجراء
dsar_subject_required = أدخل صاحب البيانات أولًا
dsar_locate_export = تحديد وتصدير
dsar_locate_export_purge = تحديد وتصدير ومحو
cancel = إلغاء
dsar_queued = في الانتظار — سيُعاد عند عودة الاتصال
dsar_previewing = معاينة…
dsar_purge_note = المحو لا رجعة فيه: يكتب شاهد قبور + مدخل سلسلة الهاش. شهادة الحذف تعيد التحقق من رأس السلسلة مباشرة.
dsar_preview_failed = فشلت المعاينة: {0}
dsar_action_failed = فشل dsar {0}: {1}
cert_fetch_failed = فشل جلب الشهادة: {0}
dsar_loading = تحميل…
dsar_back_link = ← عودة إلى أصحاب البيانات
dsar_completed_retained = {0} · {1}
dsar_subject_line = صاحب البيانات: {0}
cert_found = وُجد
cert_purged = مُسح
cert_tombstone_root = جذر شواهد القبور
cert_certified = موثق
cert_chain_head = رأس السلسلة

verdict_clean = نظيف
verdict_quarantined = محجور
edited = معدل
batch_summary = الدفعة: {0} موافق عليها · {1} محسومة مسبقًا · {2} في الانتظار (غير متصل) · {3} فاشلة
select_visible = حدد الظاهر ({0})
clear = مسح
queue_failed = فشلت القائمة: {0}
select_proposal_aria = حدد المقترح {0}
novelty_salience = الجدة {0} · البروز {1}
conflict_supersede = يتعارض مع الجزء #{0} — وافق ليُستبدل
approved_chunk = ✓ موافق عليه → الجزء #{0}
already_decided = محسوم مسبقًا
queued_offline = في الانتظار (غير متصل)
row_failed = فشل: {0}
reject_title = رفض المقترح #{0}
reingest_title = إعادة إدخال المقترح #{0} كمقترح جديد
edit_title = تعديل المقترح #{0}
post_new_proposal = نشر مقترح جديد
reason_placeholder = السبب (يسجل في سجل التدقيق)…
approve_before_deadline = وافق قبل المهلة
screen_label = الفلترة: {0}
novelty_salience_created = الجدة {0} · البروز {1} · أُنشئ {2}
approve_selected = الموافقة على المحدد ({0})
expiry_first = الانتهاء أولًا
creation_order = ترتيب الإنشاء
shortcut_hint = المفاتيح (A/S/R/E/J/K)
sample_proposal_cta = أدخل مقترحًا نموذجيًا لتجربة البوابة
cal_dismiss = تجاهل
cal_dismiss_aria = تجاهل المعايرة
suggest_reingest = اقترح إعادة إدخال
reject_modal_label = رفض بسبب
edit_modal_label = تعديل المقترح
ops_ar_counts = موافقة {0} · رفض {1}
time_until_expiry = الوقت حتى الانتهاء
expires_in = ينتهي بعد {0}
auto_reject_title = الخادم يرفض المقترحات المنتهية تلقائيًا
ops_tip_approve = موافقة (a)
ops_tip_reject = رفض (r)

connected_v = متصل — v{0} · {1}{2}
connected_capacity = مستندات {0}/{1},
could_not_reach = تعذر الوصول إلى {0}: {1}
bind_failed = فشل الربط: {0}
wizard_defaults_note = الافتراضيات فقط — قيمة الصف الصريحة تكسب دائمًا؛ هدف الربط: global
wizard_preset_aria = نموذج ملف شخصي
wizard_knob_scope = النطاق الافتراضي
wizard_knob_pii = وضع PII
wizard_knob_retention = الاحتفاظ
wizard_knob_audit = التدقيق
wizard_knob_kinds = الأنواع
wizard_knob_hold = الحجز القانوني
err_title = حدث خطأ ما
err_body = واجه العميل خطأ غير متوقع. أعد التحميل للمحاولة.
err_dismiss = تجاهل

system_title = النظام
register_title = سجل ذاكرة الوكلاء
clients_title = العملاء (لوحة BPO)
palette_open = افتح
palette_open_proposal = افتح مقترحًا
palette_open_chunk = افتح جزءًا
palette_open_entity = افتح كيانًا
palette_export_audit = صدّر التدقيق
palette_export_ump = صدّر UMP
reindex_title = إعادة الفهرسة
refresh_label = تحديث
palette_open_trace = افتح التتبع
palette_modal_label = لوحة الأوامر
palette_placeholder = اكتب أمرًا… (↑↓ للتنقل، Enter للتنفيذ، Esc للإغلاق)
palette_filter_aria = فلتر الأوامر
palette_no_match = لا تطابق

recall_placeholder = استعلم brain-server (5 أحرف على الأقل)…
recall_query_aria = استعلام الاستدعاء
recall_trace_toggle = تتبع مسار القرار
recall_min_relevance = أدنى ملاءمة
recall_rel_any = أي
recall_rel_medium_plus = متوسطة+
recall_rel_high = عالية
recall_summary = القرار: {0} · {1} نتائج
recall_trace_link = تتبع مسار القرار #{0} ↗
recall_no_hits = لا نتائج
recall_failed = فشل الاستدعاء: {0}
recall_chunk_id = الجزء #{0}
recall_score = {0} الدرجة {1}
recall_via = عبر {0}
recall_relevance = الملاءمة: {0}
recall_confidence = الثقة {0}
recall_decayed = مت decayed
recall_superseded = مستبدل
replay_back = ← عودة إلى الاستدعاء
replay_sub = مسار القرار المسجل لاستدعاء سابق (أثر تدقيق قابل لإعادة التشغيل)
replay_failed = فشل التتبع: {0}
replay_loading = تحميل…

sec_audit_chain = سلسلة التدقيق
sec_chain_ok = السلسلة سليمة
sec_chain_tampered = السلسلة فُسِدت
sec_trust_anchor = مرساة الثقة
sec_verify_chain = تحقق من سلسلة التدقيق
sec_quarantine_title = الحجر الصحي ({0})
sec_chunk_id = الجزء #{0}
sec_release = إفراج
sec_delete = حذف
sec_source = المصدر: {0}
sec_no_quarantine = لا أجزاء محجورة
sec_quarantine_failed = فشل الحجر: {0}
sec_auth_failures = إخفاقات المصادقة ({0})
sec_no_auth_failures = لا أحداث رفض مصادقة حديثة

sys_snapshot_ok = سليم
sys_snapshot_degraded = متدهور
sys_snapshot_count = {0} لقطات
sys_col_file = ملف
sys_col_size = حجم
sys_col_perms = الأذونات
sys_col_integrity = السلامة
sys_col_chain = سلسلة التدقيق
sys_perms_0600 = 0600
sys_world_readable = قابل للقراءة للجميع
sys_yes = نعم
sys_no = لا
sys_reindex_result = {0} · أُعيد تضمين {1} · تُخُطي {2}
sys_reconcile = مطابقة المصادر
sys_reconcile_result = أُقصي {0} · {1} أجزاء

register_sub = سجل مصادر قراءة فقط — من كتب كل ذاكرة وعلى ماذا استندت.
register_all = الكل
register_owner_ph = المالك…
register_source_ph = المصدر…
register_kind_ph = نوع الذاكرة…
register_failed = فشل السجل: {0}
register_empty = لا ذكريات تطابق الفلتر.
register_owner = المالك {0}
register_evidence = الأدلة
register_evidence_modal = أدلة الجزء
register_evidence_title = الأدلة — الجزء #{0}
register_src = المصدر {0}
register_rev = مراجعة {0}
register_lines = الأسطر {0}–{1}
register_ev_failed = فشلت الأدلة: {0}
register_ev_loading = تحميل الأدلة…

graph_col_entity = كيان
graph_col_depth = عمق
graph_col_domain = نطاق

data_ids_lbl = معرفات الأجزاء
data_owner_lbl = المالك
data_json = JSON
data_ump = UMP
data_ump_md = ماركداون UMP
graph_no_paths = لا مسارات
dsar_running = جارٍ…
review_sourcing_prompt = استعلام المصدر
review_approve_supersede = واستبدال
ump_verify_chain = تحقق من السلسلة
sys_multi_domains = · متعدد
health_dl_service = الخدمة
health_dl_status = الحالة
health_dl_version = الإصدار
health_dl_docs = المستندات
health_dl_rss = RSS
health_dl_capacity = السعة
health_dl_unavailable = غير متاح
health_dl_unsafe = كتل غير آمنة
health_dl_panics = حالات إنهاء ملتقطة
health_dl_corpus = المحتوى
health_dl_chunks = أجزاء
health_dl_embeddings = تضمينات
health_dl_entities = كيانات
health_dl_relationships = علاقات
health_dl_model = النموذج
health_dl_profile = الملف الشخصي
health_dl_scope = النطاق الافتراضي
health_dl_pii = وضع PII
health_dl_retention = الاحتفاظ
health_dl_audit = مستوى التدقيق
health_dl_kinds = الأنواع
health_dl_hold = الحجز القانوني الافتراضي
health_dl_note = ملاحظة
health_failed = فشلت الحالة: {0}

confirm_cancel = إلغاء
sys_http_get = GET
sys_http_post = POST
sys_http_delete = DELETE

ump_content_ph = المحتوى...
ump_query_ph = استعلام…
ump_kind_ph = النوع (اختياري)
ingest_kinds_ph = حقيقة · إجراء · خطوة · قرار
ingest_domain_ph = global
data_ids_ph = 1, 2, 3
data_owner_ph = user@example.com
data_kind_ph = حقيقة
data_days_ph = 90

approval_dock_title = الموافقات
pending_suffix = معلق
dock_empty = القائمة خالية — لا شيء ينتظر قرارًا.
dock_sla = {0} متبقية للحسم
dock_approve_aria = الموافقة على المقترح #{0}
dock_reject_aria = رفض المقترح #{0}
dock_load_failed = تعذر تحميل قائمة الموافقات.
dock_invisible_removed = أزيلت أحرف غير مرئية: {0}

runs_title = التشغيل {0}
runs_askhuman = إجابة مستحقة
runs_answer_placeholder = إجابتك على التشغيل…
runs_submit = إرسال الإجابة
runs_transcript = السجل
runs_empty = لا أحداث بعد — البث يستمع.
runs_steer = توجيه التشغيل (استشاري)
runs_steer_placeholder = توجيه لخطوة المحرك التالية…
runs_send = وجّه
runs_branches = {0} فرعًا في تاريخ هذا التشغيل
connect_needed = اتصل بـ brain-server لمتابعة هذا التشغيل.

close = إغلاق
loading = تحميل…
nav_scoreboard = لوحة النتائج
runs_timeline = الخط الزمني
runs_lineage = نسب التشغيل
runs_streaming = بث…
runs_tool_running = جارٍ
runs_tool_settled = تم
runs_tool_error = خطأ
runs_delivery = حزمة التسليم
runs_delivery_done = مكتملة
runs_handoff_title = حزمة التسليم (I-PASS)
runs_crank_label = الدورة (خطوات)
runs_crank_unwired = لا دورة HTTP بعد — شغّل `brain workflow crank <run>` حتى يصل عامل السحب.
runs_help_title = المفاتيح والأوامر
runs_help_keys = المفاتيح: J/K تنقل بين العقد · A موافقة · R رفض · ? هذه الورقة
runs_help_commands = الأوامر: /crank [steps] · /handoff · /scoreboard · /help
tl_checkpoint = نقطة تفتيش

runs_session_age = عمر الجلسة: {0} حدثًا · {1} نقاط تفتيش · الأقدم #{2}
runs_load_earlier = تحميل الأقدم
tl_branch = فرع
tl_askhuman = يحتاج بشريًا
ev_findings = النتائج
ev_contradictions = التناقضات
ev_evidence = ملخصات الأدلة
ev_questions = أسئلة التحقق
sb_title = لوحة نتائج سير العمل
sb_fcr = الحل من أول اتصال
sb_repeat = نسبة الاتصال المتكرر
sb_correctness = الصحة
sb_override = نسبة التجاوز
sb_gap = نسبة الفجوات
sb_abstention = نسبة الامتناع
sb_guidance = قبول التوجيه
sb_handoff = اكتمال التسليم
sb_escalation = الارتقاء محترم
sb_kcs_linkage = نسبة ربط KCS
sb_reuse = نسبة إعادة الاستخدام (SIR)
sb_freshness = وسيط عمر نضارة المقالات (ث)
sb_audit_ok = سلسلة التدقيق خضراء
sb_audit_notok = التدقيق غير موثق
sb_runs = {0} تشغيلًا مقيّمًا
sb_calibrated = صدر تقرير المعايرة

nav_help = المساعدة والاختصارات
help_close = إغلاق المساعدة
