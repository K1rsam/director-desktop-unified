# 20261020 源接入本地兼容补丁

来源：本项目《战争打了六亿年》接入诊断，经主控授权修复可据源明确的机械分型。未修改冻结原文、分集source.txt或上游TypeScript源。

补丁文件：`20261020-source-head-compat.patch`。基线与当前SHA-256、补丁SHA-256见上层`provenance.json`的`localPatches`。原`sourceSha256`与`generatedSha256`继续表示原始上游/生成基线，新增`packagedSha256`明确当前本地包；不能声称当前文件仅由原生成命令得到。

变更：

- planner仅将已在人物名单/knownSpeakers确认的`姓名（OS/VO）原句`缺冒号布局识别为对白；陌生姓名仍报`UNKNOWN_SPEAKER`。
- 现有有限原声说明谓词增加`换新约／工种变更／先看清／三件事／先算费用`。仍需行首有据说者；不把任意内部冒号自动当一句，也不压掉真实多说者。
- 一个明确OS/VO位于括号内两个提示之间时，用两个原文cue切片及独立channelSource保存；编译器与独立标准source验证同步核两段精确范围。只有对应原头的两段可跨该声道间隙，任意正文缺口仍拒绝。
- 包内编译器允许完整明确OS/VO括号头紧接正文，不补冒号；来源证据仍完整覆盖原物理行。

修复未改变`prepare-source.mjs`；此前同集双标题、显式集名/闪回结构映射保留。第92集冻结源的串句仍产生诊断，不替用户编排。第92集独立解释本由主控另行处理，不属于此补丁。

复现：从provenance所列原生成/运行基线开始，以`git -c core.autocrlf=false apply PATCH`应用同名unified diff，保持原基线及当前文件的LF换行，再核每个`afterSha256`。回滚使用`git -c core.autocrlf=false apply -R PATCH`并核`beforeSha256`；四文件正向与反向重放均已实测原始字节hash一致。如果本机默认`core.autocrlf`把输出写回CRLF，不可据文本相同宣称字节相同，应使用上述禁转换命令重放。`generate.mjs`本身只再生上游基线，之后必须重新应用本补丁，不能直接把再生文件当当前已修包。运行`node --test tests/intake.test.mjs`核回归与来源hash。

验证边界：机械保真只证明来源文本、UTF-16范围、说者/声道/cue映射与完整归属；`semantic:NOT_RUN`保留，不宣称剧情语义审定、分镜质量或用户验收。
