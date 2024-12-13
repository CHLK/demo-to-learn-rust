来源：https://github.com/tikv/tikv/pull/17784

本文没有特别说明的话都是针对resizable_threadpool_v2.rs进行讲述。
# 关键词
基础：Option
生命周期：可变/不可变借用，所有权，'static
智能指针：Box，Arc，Weak
函数式编程：函数：FnOnce、FnMut、Fn，闭包：||{...}、move ||{..}
多线程和并发编程：Mutex，AtomicUsize，Send + Sync
异步编程：async，Future，Tokio Runtime，async move { ... }
范型和特征：特征约束（Fut: Future<Output = ()> + Send + 'static），Drop，Future
返回值及错误处理：anyhow，Result
三方库：tokio，tracing，anyhow
# 设计背景
resizable_threadpool_v2.rs 实现了一个可调整大小的异步任务运行时（ResizableRuntime），它允许动态地改变线程池中线程的数量。这个运行时使用了Tokio库，它是Rust中最流行的异步运行时之一。
它经历过一次改版，改版前代码为：resizable_threadpool_v1.rs 。
做这个修改原因Issue是：https://github.com/tikv/tikv/issues/17807 ，简单来说v1版本resizable runtime在销毁时可能会发生阻塞，具体的表现可以看看v1、v2中的test_drop()测试方法。
发生上述问题的主要原因是：在resizable runtime销毁后，仍能继续提交任务并执行。
# 原理
## DaemonRuntime
DaemonRuntime 结构体封装了一个 Tokio 的 Runtime 实例以及一个 TaskTracker 用于跟踪任务的状态。它提供了 spawn 方法来启动新的异步任务，并且实现了 Drop trait 来确保当 DaemonRuntime 被丢弃时，相关的资源会被正确清理。

## DaemonRuntimeHandle
DaemonRuntimeHandle 是对 DaemonRuntime 的引用计数弱引用，允许安全地克隆并传递给其他线程或任务。它提供了 spawn 和 block_on 方法，可以用来启动新任务或阻塞当前线程直到某个异步操作完成。

## ResizableRuntime
ResizableRuntime 是这段代码的核心结构体，它管理着一个可以调整大小的线程池。它包含了以下内容：

size: 当前线程池的大小。
version: 线程池版本号，用于命名线程。
thread_prefix: 线程名称前缀。
gc_runtime: 用于清理旧的 Runtime 实例。
current_runtime: 当前正在使用的 Runtime 实例。
replace_pool_rule: 一个闭包，定义了创建新的 Runtime 实例的规则。
after_adjust: 一个闭包，在调整线程池大小后执行一些额外的动作。
ResizableRuntime 提供了 new 方法来创建一个新的实例，并有 adjust_with 方法来调整线程池的大小。adjust_with 方法会创建一个新的 Runtime 实例，替换掉旧的实例，并将旧的实例放入 gc_runtime 中等待被清理。
# FAQ
## 问什么需要增加TaskTracker
### TaskTracker介绍
通常与 CancellationToken 一起使用来实现优雅关闭。 CancellationToken 用于向任务发出信号，告知它们应该关闭，而 TaskTracker 用于等待它们完成关闭。
TaskTracker 还将跟踪 closed 布尔值。这用于处理 TaskTracker 为空的情况，但我们还不想关闭。这意味着 wait 方法将等待，直到以下两种情况同时发生：
- 必须使用 close 方法关闭 TaskTracker；
- TaskTracker 必须为空，即它正在跟踪的所有任务都必须已退出。

当对 wait 的调用返回时，可以保证所有跟踪的任务都已退出，并且 future 的析构函数已完成运行。但是，JoinHandle::is_finished 可能会在短时间内返回 false。
### 原因
- 在resize后优雅关闭旧的Runtime，保证旧的Runtime中的任务能正常完成；如没有TaskTracker，RunTime销毁时会强制清理所有关联资源（如线程，任务等），如果任务没有执行完成会被强制关闭；
- TaskTracker 的 close 方法会阻止新的任务加入到跟踪列表，并且 wait 方法会等待所有现存的任务完成，之后就可以安全地丢弃旧的 Runtime 实例了；
## 为什么DaemonRuntimeHandle中要使用Weak而不是Arc
防止DaemonRuntime销毁受阻碍，若使用Arc的话，由于Arc是强引用，存在DaemonRuntimeHandle使用Arc引用的DaemonRuntime时，由于DaemonRuntime有强引用，导致它不会被释放。使用Weak在Drop时，由于是弱引用，DaemonRuntime的释放不会受到有多个DaemonRuntimeHandle引用的影响。
## 为什么需要ResizableRuntime::handle方法
生成一个DaemonRuntimeHandle，使用Weak关联DaemonRuntime，使得在resize或ResizableRuntime销毁时，ResizableRuntime::spawn和ResizableRuntime::block_on中即使获取到的DaemonRuntimeHandle引用的是老的DaemonRuntime或需要销毁的ResizableRuntime中的DaemonRuntime，也不会阻碍对应的DaemonRuntime（resize时为老的、ResizableRuntime销毁时为ResizableRuntime引用的）销毁。在DaemonRuntime销毁后，DaemonRuntimeHandle会丢弃提交的task，并打印一条error日志。
## Fut: Future<Output = ()> + Send + 'static 中为什么为Future添加 'static 约束
- 生命周期安全：保证 Future 可以在任意线程上执行，并且不会因为持有短期引用而导致悬空指针等问题。
- 所有权清晰：确保 Future 不依赖于外部上下文，除了明确传递给它的数据，这有助于简化所有权关系。
- 虽然 'static 确保了 Future 不包含短期引用，但这并不意味着它会永远存在。相反，它只是确保 Future 在整个程序的生命周期内都是有效的，或者至少没有对其他对象的短期引用。当满足以下条件时，Future 仍然会被正常释放：
  - 它已经完成了执行；
  - 它被显式取消了；
  - 它达到了某个预设的超时时间；
  - 它所在的运行时被销毁了。
## resizable_threadpool使用最佳实践
- 提交的Future需要正确的xian响应取消信号，是的旧的Runtime销毁或threadpool销毁时，其中的所有任务能正确退出
  - 使用 tokio::select! 宏来监听多个异步操作，包括一个取消信号。
  - 定期检查全局取消标志。
  - 使用带有超时的时间逻辑。
- 不要在这个threadpool提交长期运行的任务，由于增加了TaskTracker，在resize之后，旧的Runtime销毁时，这些任务也会被强制关闭，导致无法正常退出。
