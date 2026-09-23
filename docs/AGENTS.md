# AGENTS.md — docs(契约文档体系)

`00-overview.md` 是文档地图(只收地基文档 01–09);`01`/`02`/`03` 是契约(FR 编号、架构、协议字节级规格),改它们 = 跨端契约改动,纪律见根 `AGENTS.md`。

## 写作约定

- 一个交付物一份 `NN-<主题>.md`:背景 → 做法 → 取舍 → 实测数字 → **未验清单**(CONTRIBUTING §6)。
- **历史文档不改写**:结论翻转时用「日期 + 补充/更正块」追加(先例 `02-architecture.md`、`12-m1-device-acceptance.md` §11);整个文档过时则在头部挂「历史记录说明」并指向现状文档(先例 `69-playout-latency-recovery.md` → `72-remove-pairing.md` 式)。
- 诚实记账:「未验」「这次没通过」「代价判据不达标」都要写进文档与看板,不只写好消息;真机证据原始件落 `target/evidence/`(不入库),文档里引用其路径与关键数字。
- 编号 `NN` 递增、不复用空号(14/44 是历史空号);同一主题重写开新号(先例 71/72 remove-pairing),新文档用下一个未占用的号。
- 子目录:`manual/` 用户手册(用户可见行为变化要同步,CONTRIBUTING §3);`compliance/` 许可合规产物;`design/` 是探索稿,**不代表现状**,别当依据引用(桌面的冻结设计语言在 `desktop/index.html` 头部注释)。
