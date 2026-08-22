# Frozen Eval Query Fixture (v1.28.6 expanded set)

This file is the **judged query set** for the eval harness: 106 queries over
the 25-doc frozen corpus in `tests/eval.rs` (`DOCS`, indices below). The
pre-1.28.6 starter set was 37 queries over 10 docs — a wiring fixture, not
quality evidence. This is the close-out frozen set; changes require a new
dataset hash recorded in `BENCHMARKS.md`.

## Corpus (`DOCS`, 0-based)

| 0 | Bignay is a tropical fruit and a good alternative to blueberry, rich in antioxidants. |
| 1 | The Rust programming language guarantees memory safety without a garbage collector. |
| 2 | Vitamin D3 supplementation improves immune function and bone density in deficient adults. |
| 3 | The GDPR is a European regulation protecting the personal data of EU residents. |
| 4 | Gut microbiome diversity affects inflammation markers and immune system regulation. |
| 5 | SQLite is an embedded relational database with FTS5 full-text search support. |
| 6 | ISO 9001 is the international standard for quality management systems. |
| 7 | Ownership and borrowing are Rust's core concepts for compile-time memory safety. |
| 8 | Antioxidants in tropical fruits like bignay help reduce oxidative stress. |
| 9 | The GDPR covers any organization processing EU residents' data, with fines up to four perc |
| 10 | VxRail LCM upgrades require a green RCM release certification manifest before any upgrade  |
| 11 | A stretched-cluster rolling reboot reboots one ESXi node at a time; never reboot two nodes |
| 12 | vSAN storage policies set FTT failures to tolerate and FTM failure tolerance method per vi |
| 13 | PowerFlex protection domains map fault sets to failure boundaries across SDS storage pools |
| 14 | NSX-T managers push micro-segmentation firewall rules to transport nodes over the control  |
| 15 | A DPA data processing agreement under GDPR Article 28 binds the processor to the controlle |
| 16 | Standard Contractual Clauses 2021 are the approved EU transfer mechanism for processors ou |
| 17 | RA 10173 the Philippine Data Privacy Act requires NPC breach notification within 72 hours. |
| 18 | Schrems II requires a transfer impact assessment before any personal-data transfer to a th |
| 19 | Legal holds freeze erasure until every hold is explicitly released by the operator. |
| 20 | Intermittent storage fabric latency usually traces to a failing SFP on one uplink port, no |
| 21 | High VM disk latency triage order: vSAN backend congestion, then host cache, then the phys |
| 22 | A node flapping out of vCenter management is most often NTP drift breaking certificate val |
| 23 | PSOD purple diagnostic screen dumps land in var log and must be collected before any reboo |
| 24 | vMotion failing at ten percent points to VMkernel port mobility or a missing shared datast |

## Vertical gold set — migration

### Q1 — [migration]
Query: "RCM green before upgrade wave"
Relevant: [10]

### Q2 — [migration]
Query: "release certification manifest VxRail"
Relevant: [10]

### Q3 — [migration]
Query: "stretched cluster rolling reboot"
Relevant: [11]

### Q4 — [migration]
Query: "reboot one node at a time"
Relevant: [11]

### Q5 — [migration]
Query: "vSAN policy failures to tolerate"
Relevant: [12]

### Q6 — [migration]
Query: "FTT FTM storage policy"
Relevant: [12]

### Q7 — [migration]
Query: "PowerFlex fault sets boundaries"
Relevant: [13]

### Q8 — [migration]
Query: "protection domain SDS pools"
Relevant: [13]

### Q9 — [migration]
Query: "micro-segmentation rules transport nodes"
Relevant: [14]

### Q10 — [migration]
Query: "NSX-T control plane firewall"
Relevant: [14]

### Q11 — [migration]
Query: "LCM wave scheduling prerequisite"
Relevant: [10]

### Q12 — [migration]
Query: "ESXi concurrent reboot rule"
Relevant: [11]

### Q13 — [migration]
Query: "upgrade planning checklist"
Relevant: [10]

### Q14 — [migration]
Query: "maintenance window node procedure"
Relevant: [11]

### Q15 — [migration]
Query: "per-VM resilience settings"
Relevant: [12]

### Q16 — [migration]
Query: "scale-out storage fault domains"
Relevant: [13]

### Q17 — [migration]
Query: "distributed firewall distribution"
Relevant: [14]

### Q18 — [migration]
Query: "one at a time not parallel"
Relevant: [11]

### Q19 — [migration]
Query: "FTT"
Relevant: [12]

### Q20 — [migration]
Query: "RCM"
Relevant: [10]

### Q21 — [migration]
Query: "upgrade prerequisite manifest check"
Relevant: [10]

### Q22 — [migration]
Query: "cluster maintenance single node rule"
Relevant: [11]

### Q23 — [migration]
Query: "storage policy tolerance settings"
Relevant: [12]

### Q24 — [migration]
Query: "fault boundary mapping storage"
Relevant: [13]

### Q25 — [migration]
Query: "firewall rule propagation mechanism"
Relevant: [14]


## Vertical gold set — legal

### Q26 — [legal]
Query: "processor bound to controller instructions"
Relevant: [15]

### Q27 — [legal]
Query: "Article 28 data processing agreement"
Relevant: [15]

### Q28 — [legal]
Query: "EU transfer mechanism outside EEA"
Relevant: [16]

### Q29 — [legal]
Query: "standard contractual clauses 2021"
Relevant: [16]

### Q30 — [legal]
Query: "Philippine breach notification deadline"
Relevant: [17]

### Q31 — [legal]
Query: "NPC 72 hours data privacy act"
Relevant: [17]

### Q32 — [legal]
Query: "transfer impact assessment requirement"
Relevant: [18]

### Q33 — [legal]
Query: "third country data transfer ruling"
Relevant: [18]

### Q34 — [legal]
Query: "hold freezes erasure until released"
Relevant: [19]

### Q35 — [legal]
Query: "legal hold explicit release"
Relevant: [19]

### Q36 — [legal]
Query: "who signs the processing agreement"
Relevant: [15]

### Q37 — [legal]
Query: "cross-border contract for vendors"
Relevant: [16]

### Q38 — [legal]
Query: "manila privacy regulator timeline"
Relevant: [17]

### Q39 — [legal]
Query: "assessment before moving data abroad"
Relevant: [18]

### Q40 — [legal]
Query: "can we delete while litigation pending"
Relevant: [19]

### Q41 — [legal]
Query: "without EEA adequacy what mechanism"
Relevant: [16, 18]

### Q42 — [legal]
Query: "erasure blocked during investigation"
Relevant: [19]

### Q43 — [legal]
Query: "DPA"
Relevant: [15]

### Q44 — [legal]
Query: "controller processor contract terms"
Relevant: [15]

### Q45 — [legal]
Query: "EEA exit data transfer compliance"
Relevant: [16, 18]

### Q46 — [legal]
Query: "philippines data privacy regulator"
Relevant: [17]

### Q47 — [legal]
Query: "schrems assessment obligation"
Relevant: [18]

### Q48 — [legal]
Query: "release of frozen records operator"
Relevant: [19]


## Vertical gold set — troubleshoot

### Q49 — [troubleshoot]
Query: "storage latency failing SFP uplink"
Relevant: [20]

### Q50 — [troubleshoot]
Query: "intermittent fabric latency cause"
Relevant: [20]

### Q51 — [troubleshoot]
Query: "VM disk latency triage order"
Relevant: [21]

### Q52 — [troubleshoot]
Query: "vSAN congestion host cache check"
Relevant: [21]

### Q53 — [troubleshoot]
Query: "node flapping out of vCenter"
Relevant: [22]

### Q54 — [troubleshoot]
Query: "NTP drift certificate validation"
Relevant: [22]

### Q55 — [troubleshoot]
Query: "purple screen dump collection"
Relevant: [23]

### Q56 — [troubleshoot]
Query: "PSOD logs before reboot"
Relevant: [23]

### Q57 — [troubleshoot]
Query: "vMotion fails at ten percent"
Relevant: [24]

### Q58 — [troubleshoot]
Query: "VMkernel shared datastore check"
Relevant: [24]

### Q59 — [troubleshoot]
Query: "flaky network port hardware swap"
Relevant: [20]

### Q60 — [troubleshoot]
Query: "slow virtual machine diagnostics"
Relevant: [21]

### Q61 — [troubleshoot]
Query: "host lost from inventory causes"
Relevant: [22]

### Q62 — [troubleshoot]
Query: "kernel panic evidence preservation"
Relevant: [23]

### Q63 — [troubleshoot]
Query: "live migration stuck early stage"
Relevant: [24]

### Q64 — [troubleshoot]
Query: "PSOD"
Relevant: [23]

### Q65 — [troubleshoot]
Query: "SFP"
Relevant: [20]

### Q66 — [troubleshoot]
Query: "NTP drift"
Relevant: [22]

### Q67 — [troubleshoot]
Query: "vMotion"
Relevant: [24]

### Q68 — [troubleshoot]
Query: "uplink port errors hardware fault"
Relevant: [20]

### Q69 — [troubleshoot]
Query: "disk performance investigation steps"
Relevant: [21]

### Q70 — [troubleshoot]
Query: "time sync breaks management connection"
Relevant: [22]

### Q71 — [troubleshoot]
Query: "collect diagnostics before restart"
Relevant: [23]

### Q72 — [troubleshoot]
Query: "shared datastore requirement migration"
Relevant: [24]


## Cross-category additions (incl. the original 37-query smoke set)

### Q73 — [general]
Query: "blueberry alternative fruit"
Relevant: [0, 8]

### Q74 — [general]
Query: "memory safe programming language"
Relevant: [1, 7]

### Q75 — [general]
Query: "vitamin supplements immune health"
Relevant: [2]

### Q76 — [general]
Query: "EU data protection regulation"
Relevant: [3, 9]

### Q77 — [general]
Query: "gut inflammation microbiome"
Relevant: [4]

### Q78 — [general]
Query: "embedded database search"
Relevant: [5]

### Q79 — [general]
Query: "quality management standard"
Relevant: [6]

### Q80 — [general]
Query: "GDPR organization coverage"
Relevant: [3, 9]

### Q81 — [general]
Query: "antioxidants tropical fruit stress"
Relevant: [0, 8]

### Q82 — [general]
Query: "Rust ownership borrowing"
Relevant: [1, 7]

### Q83 — [general]
Query: "which fruit lowers oxidative damage"
Relevant: [0, 8]

### Q84 — [general]
Query: "garbage-collector-free memory model"
Relevant: [1, 7]

### Q85 — [general]
Query: "bone density supplement recommendation"
Relevant: [2]

### Q86 — [general]
Query: "who enforces European privacy law"
Relevant: [3, 9]

### Q87 — [general]
Query: "diet and immune system link"
Relevant: [4]

### Q88 — [general]
Query: "in-process database with text search"
Relevant: [5]

### Q89 — [general]
Query: "international quality certification"
Relevant: [6]

### Q90 — [general]
Query: "compile-time safety guarantees Rust"
Relevant: [1, 7]

### Q91 — [general]
Query: "tropical superfruit antioxidants"
Relevant: [0, 8]

### Q92 — [general]
Query: "privacy fines percentage of revenue"
Relevant: [3, 9]

### Q93 — [general]
Query: "NOT a cloud database embedded engine"
Relevant: [5]

### Q94 — [general]
Query: "no garbage collector language"
Relevant: [1, 7]

### Q95 — [general]
Query: "GDPR"
Relevant: [3, 9]

### Q96 — [general]
Query: "Kubernetes ingress"
Relevant: []

### Q97 — [general]
Query: "database without a network service"
Relevant: [5]

### Q98 — [general]
Query: "language chosen for systems reliability"
Relevant: [1, 7]

### Q99 — [general]
Query: "supplement for immune deficiency adults"
Relevant: [2]

### Q100 — [general]
Query: "fruit similar to blueberry nutrition"
Relevant: [0, 8]

### Q101 — [general]
Query: "microbiome diversity inflammation markers"
Relevant: [4]

### Q102 — [general]
Query: "full text search embedded sqlite"
Relevant: [5]

### Q103 — [general]
Query: "ISO certification for factories"
Relevant: [6]

### Q104 — [general]
Query: "borrow checker memory safety"
Relevant: [1, 7]

### Q105 — [general]
Query: "bignay health benefits"
Relevant: [0, 8]

### Q106 — [general]
Query: "EU residents personal data rules"
Relevant: [3, 9]

