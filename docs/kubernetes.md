# Kubernetes

[返回使用手册](manual.md#kubernetes)

Kubernetes 是远程终端右侧栏的一个工具。集群来自 **SSH 主机**上的 kubeconfig，不是运行 ShellRS 的电脑。连接到能访问集群的管理主机后，在右侧选择 Kubernetes。

## 查询范围

- **集群（context）**：读取 `kubectl config current-context` 和 `kubectl config get-contexts -o name`，支持选择其他 context。所有 API 请求带 `--context`，不会执行 `use-context` 或改动 kubeconfig。
- **命名空间**：通过 `kubectl get namespaces -o json` 读取完整列表，名称排序。默认“全部命名空间”，API 请求使用 `--all-namespaces`；选择一个 namespace 后，请求改为明确的 `--namespace`。
- **资源类型**：Pod、Deployment（deploy）、StatefulSet（sts）、DaemonSet、ReplicaSet、Service（svc）、Ingress、Job、CronJob、PVC。
- **搜索**：在当前类型和命名空间查询结果中，按名称、命名空间、状态、标签进行不区分大小写的过滤，不额外执行服务器命令。

列表分页交给 kubectl 处理（`--chunk-size=200`），不会仅显示第一页。共享 SSH 查询通道每条命令最多运行 10 秒，输出上限 1 MB；Kubernetes API 请求超时为 8 秒。大集群的资源 JSON 超过上限时，会明确报告数据不完整，需选择更小的命名空间范围；不会显示解析到一半的资源。

## 资源详情与 Pod

点击资源名称打开详情，重新读取该资源，显示状态与配置、创建时间、标签、容器与镜像、完整 JSON。JSON 是只读文本，可选择复制。

右上角“关闭”、Esc 或点击窗口外侧均可关闭详情，随后焦点回到资源列表。查询尚未完成时也可以关闭；迟到的回复不会重新打开窗口。

Deployment、StatefulSet、DaemonSet、ReplicaSet、Job 和带选择器的 Service 会按 `spec.selector` 查询同命名空间内的 Pod，支持 `matchLabels` 和 `matchExpressions`。点击关联 Pod 后，详情页切换到该 Pod，再进入日志页。没有直接 Pod 选择器的资源（例如 CronJob 或 ExternalName Service）可回到 Pod 列表按名称或标签筛选。

## 容器日志与检索

- 容器列表包括 `containers`、`initContainers` 和 `ephemeralContainers`。优先选择 Pod 的默认容器注解指定的容器，否则选择第一个普通容器。
- 最近行数：100、500、1000（默认）、5000、10000。
- 时间范围：5m、15m、1h（默认）、6h、24h。
- “上一次容器”对应 `kubectl logs --previous`，用于查看所选容器上一次终止前的日志。
- 每次读取最多 512 KB，带时间戳；切换容器、行数、时间范围或上一次容器选项时，清除旧范围的快照并重新读取。
- 检索区分大小写；默认按字面文本匹配，也可启用正则表达式。不合法的正则会显示错误。检索显示“匹配行数 / 已读取行数”，不会把搜索文本拼到 shell 命令里。
- GUI 显示按需读取的快照，不定时轮询或持续跟踪，也不接入 Loki、Elasticsearch 等外部日志索引。要持续跟踪可复制“实时跟踪日志”命令到终端。

## 生成命令

详情页的“生成命令”提供：查询状态与事件（describe）、查询 YAML、查询容器日志、实时跟踪日志、进入容器（`kubectl exec -it … -- /bin/sh`）。每个命令都包含当前 context、namespace 和资源名，Pod 命令还包含所选容器；日志命令沿用当前日志范围选项。复制后可在这台 SSH 主机的终端执行，容器需要有 `/bin/sh` 才能使用该进入命令。

## 权限与失败处理

使用 kubeconfig 中现有的身份和 RBAC 权限。需要集群级 `list namespaces`；浏览需要相应资源的 `list`，详情需要 `get`，关联 Pod 需要 `list pods`，日志需要 `get pods/log`。复制 exec 命令不请求集群权限；用户执行时还需要 `create pods/exec`。

工具本身只查询，不修改集群或保存 kubeconfig、token、客户端证书。namespace 查询被拒绝时，显示实际的 Forbidden 错误并保留 context 选择；不会把没有权限的 namespace 当成不存在。kubectl 未安装、context 无效、API 不可达、SSH 断开或数据被截断也会显示原因，可以修复后点击刷新。

## 开发与测试

`src/kubernetes/` 独立于 `workspace`：`commands.rs` 负责命令、退出状态封装；`model.rs` 负责纯解析、摘要、Pod 选择器；`panel.rs` 持有查询范围和列表；`details.rs` 持有固定查询目标的资源详情和日志。所有用户命令经 `app::KubernetesAction` 路由到 `Workspace::on_kubernetes_action`。

使用共享 `ExecTarget` 在远程终端自己的 SSH 连接上执行，不建立第二次登录。stderr 与退出状态经过封装，因为共享 Exec API 只返回 stdout。实际执行脚本编码为 POSIX `printf %b` 支持的八进制字符串，避免登录 shell 是 fish/csh 时处理嵌套引号或历史 `!`；生成给人的命令仍是可读的 kubectl 命令。所有来自 kubeconfig、API 的参数都有 shell 引用。

异步结果按代次验证，改变主机或查询范围会取消等待并拒绝旧结果。对话框绑定打开时的 SSH 连接、context、namespace 和资源；关闭对话框后不保留实体。后台回复由共享终端的 UI 轮询处理，不由工作线程唤醒 GPUI。

验证命令：

```sh
cargo test --lib kubernetes::
cargo test --test ui kubernetes::
cargo test --lib i18n::
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

UI 测试在真实 Workspace 中点击工具、操作 namespace 与资源类型下拉框、打开详情、导航关联 Pod、切换容器、查询上一次容器日志、检索和复制命令。SSH 回复完全注入，不读取开发者 kubeconfig，不连接真实集群，不访问钥匙串。Unix 单元测试还通过项目的 `testing::sh` 验证实际 shell 执行、stderr 和退出状态。
