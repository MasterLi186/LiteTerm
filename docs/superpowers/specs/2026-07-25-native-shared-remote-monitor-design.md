# Native 共享远端监控设计

## 目标与边界

Native 侧栏必须跟随活动标签显示正确主机的 CPU、内存、交换区、进程、网络、磁盘、运行时间和负载。监控身份使用 `user@host:port`：

- 本地标签统一使用 `Local`。
- 相同 `user@host:port` 的 SSH 标签共享一条远端监控连接、最新快照和网络历史。
- 用户、主机或端口任一不同即建立独立监控。
- 文件管理器继续按 `TabId` 持有独立 SFTP worker、路径、列表和传输状态，不使用监控身份。

## 架构

新增可哈希的 `MonitorKey::{Local, Remote { user, host, port }}`。`App` 保存：

- `monitor_cache: HashMap<MonitorKey, MonitorData>`
- `remote_monitors: HashMap<MonitorKey, RemoteMonitorHandle>`
- 每个 key 独立的侧栏网络接口选择和折线历史

本机采集线程继续使用 `MonitorCollector`，但事件携带 `MonitorKey::Local`。SSH 会话连接成功后，App 根据标签参数确保对应远端 worker 存在；复制相同 SSH 标签只增加使用者，不创建新 worker。关闭标签后重新统计仍在使用的远端 key，最后一个使用者消失时发送非阻塞关闭信号并移除缓存。

## 远端采集与数据流

远端 worker 使用独立 SSH 连接，避免监控命令阻塞终端或 SFTP。每两秒通过一个带分段标记的只读命令采集 `/proc/stat`、`/proc/meminfo`、`/proc/net/dev`、`/proc/loadavg`、`/proc/uptime`、`/proc/cpuinfo`、`df` 和 `ps`。解析逻辑与 I/O 分离，以固定文本样本测试。

worker 事件携带 `MonitorKey` 和 generation。App 只接受当前 generation 的数据，写入对应缓存；活动标签渲染时只读取其 key。切换到尚无快照的 SSH 标签时显示“正在采集”，不得继续显示本机或上一远端数据。

## 生命周期与错误处理

远端连接、认证或命令失败只将该 key 标记为错误，不影响终端和文件管理器。界面显示简短状态并允许后续已连接标签触发重试。所有 TCP 和通道操作设置超时；关闭标签和窗口时只发送 shutdown，不在 UI 线程等待。密码不得进入 key、日志或 `Debug` 输出。

## 测试与验收

- 单元测试覆盖 key 等价性、不同远端隔离、标签到 key 的映射及最后使用者释放。
- 解析测试覆盖正常、缺段、异常数值和网络速率差分。
- 状态测试覆盖本地→SSH、SSH A→SSH A 副本、SSH A→SSH B、缓存未就绪和过期 generation。
- 生命周期测试验证复制标签只创建一个 worker，文件管理器仍按两个 `TabId` 独立。
- 手工验收切换与复制标签时侧栏立即切换，关闭最后一个同主机标签后监控连接和 FD 被释放。
