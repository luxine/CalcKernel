# 实施期设计复诊 06：记录的 argv[0] 必须等于子进程实际 argv[0]

## 阻断复诊

命令证据复核发现，Clang/Rust oracle 为保持其安装目录中的 resource/sysroot 发现能力，
需要由原安装路径执行；首版 collector 因而把记录的 evidence-relative `argv[0]` 替换为
原绝对路径后启动进程。虽然两处可执行文件字节相同，报告中的 `Command.argv` 却不再是
实际传给子进程的字符串向量，违反 exact argv 契约，属于证据绑定阻断。

## 修订决议

Collector 始终原样传递 report 中的完整 `argv`。对于需要原安装布局的 Clang、Rustc
或 replay compiler，它通过操作系统的 executable 参数选择已由 checker 证明字节等同的
原可执行映像，而不改写进程看到的 `argv[0]`；普通候选和受监督命令直接执行记录路径。
Checker 继续验证 retained identity、live original digest/version/resource runtime 与 argv[0]
映射，三者共同闭合“执行映像身份 + 精确参数向量 + 安装布局”。

## 验证与门槛

本地真实 Clang 探针以 evidence-relative `argv[0]`、原映像 executable、空环境成功生成
目标文件。该修订修复命令真实性，不改变任何性能、正确性、资源或 CI 门槛。
