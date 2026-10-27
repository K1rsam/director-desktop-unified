# v6.5.3 台词核源与后期隔离

本地版本：20261023-v6.5.3-dialogue-postproduction。上版v6.5.2发布提交eae795760b8f2d11296d388c75c73e6c8b4fcf9a；本轮本地维护，尚未上传GitHub。

- 原话逐字、说者与声道保留；增强演法、提速与声窗调整不得改词。疑句回原件，不让下游重写source。
- 接入方在创作前freeze-source，bundle绑定sourceLockHash；check/export/verify带独立源锁，对照完整source后再核dialogueParts原句拼接。源与分镜同时改词也拒绝。全剧锁可供逐集与最终bundle复用；旧无锁稿保留回查且明确bundle-only证据，不重新生产。
- 人名条及指定后期叠字只交独立postproduction-notes.md；生成通用提示词、头部、初末态、六栏与最终MD不写其内容或后期添加占位话。源行不删，不改成旁白，实际场内文字、系统面板与源对白保留。
- 新生成文本声明generationTextVersion:"voice-delivery-v1"，不再自动投影原始q.cue/performanceCues，堵住后期说明伪装成原声提示的出口。真正源表演由作者落实既有delivery/picture/sound，整编与非作者按说者和句内位置逐项回读，不凭字段非空声称保全。历史无该策略的投影仍按原协议回查。
- 保留v6.5.2实际动作出点/入点与cut_pairs审读，不引入网页、数据库或队列依赖。

有源锁的本地机械检查证明与创作前接入文本一致，不证明原件接入绝对正确、导演内容质量、Seedance实际生成或视频效果。后期隔离仍需作者与非作者读回最终正文，不用关键词黑名单或静默删字冒充语义审查。
