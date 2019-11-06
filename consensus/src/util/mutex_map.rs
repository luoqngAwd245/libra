use futures::compat::Future01CompatExt;
use std::{
    collections::HashMap,
    hash::Hash,
    sync::{self, Arc},
};

type AsyncMutex = futures_locks::Mutex<()>;

impl<T> Drop for MutexMapGuard<T>
where
    T: Hash + Eq + Clone,
{
    fn drop(&mut self) {
        // Drop guard: release lock, decrement internal reference counter on lock
        self.mutex_guard
            .take()
            .expect("ExistGuard should have mutex_guard on drop");
        self.owner.guard_dropped(&self.key);
    }
}

/// MutexMap acts as a map of mutexes, allowing lock based on some key
///
/// More specifically MutexMap::lock(key) returns lock guard, that is guaranteed to be exclusive
/// for given key. Until returned guard is dropped, consequent calls to MutexMap::lock with same
/// key going to be blocked.
///
/// MutexMap is from a 'future locks' family, meaning that lock() method does not block thread,
/// but instead returns Future(implicitly, through 'async') that is only fulfilled when lock
/// can be acquired.
/// MutexMap充当互斥锁的映射，允许基于某些键进行锁定
///
/// 更具体地说，MutexMap :: lock（key）返回锁定保护，保证对于给定的密钥是唯一的。 直到返回的防护被删除，随后调用MutexMap :: lock同样
/// 密钥将被阻止。
///
/// MutexMap来自'future locks'系列，这意味着lock（）方法不会阻塞线程，而是返回Future（隐式地，通过'async'），
/// 只有在可以获取锁时才能实现。
pub struct MutexMap<T>(Arc<sync::Mutex<HashMap<T, AsyncMutex>>>)
where
    T: Hash + Eq + Clone;

/// This guard is returned by MutexMap::lock, dropping this guard releases lock
/// 这个守卫是由MutexMap :: lock返回的，放弃了这个守卫释放锁定
pub struct MutexMapGuard<T>
where
    T: Hash + Eq + Clone,
{
    mutex_guard: Option<futures_locks::MutexGuard<()>>,
    owner: MutexMap<T>,
    key: T,
}

// Implementation overview:
// MutexMap maintains map of future mutexes
// Map itself is protected under regular, non-future mutex, to reduce code complexity.
// Operation on this map is fast, so having traditional lock does not have impact.
//
// Only private methods get_or_create_mutex and guard_dropped acquire lock on internal map
// 实施概述：
//MutexMap维护未来互斥锁的映射
//映射本身受常规，非未来互斥锁的保护，以降低代码复杂性。
//在这张地图上的操作很快，因此传统的锁没有影响。
//
//只有私有方法get_or_create_mutex和guard_dropped才能在内部地图上获取锁定
impl<T> MutexMap<T>
where
    T: Hash + Eq + Clone,
{
    /// Creates new, empty MutexMap
    /// 创建新的空MutexMap
    pub fn new() -> MutexMap<T> {
        MutexMap(Arc::new(sync::Mutex::new(HashMap::new())))
    }

    /// async method is fulfilled when lock can be acquired
    /// Lock is released when returned guard is dropped
    /// 可以获取锁定时实现异步方法当退回保护时，锁定被释放
    pub async fn lock(&self, key: T) -> MutexMapGuard<T> {
        let mutex_fut = self.get_or_create_mutex(key.clone());
        let mutex_guard = mutex_fut.compat().await.unwrap();
        let owner = Self(Arc::clone(&self.0));
        MutexMapGuard {
            mutex_guard: Some(mutex_guard),
            owner,
            key,
        }
    }

    /// Creates new empty lock_set for given map.
    /// Use this if you want to acquire multiple locks in a loop in single task
    /// 为给定的map创建新的空lock_set。
    /// 如果要在单个任务中获取循环中的多个锁，请使用此选项
    pub fn new_lock_set(&self) -> LockSet<T> {
        LockSet::new(self)
    }

    fn get_or_create_mutex(&self, key: T) -> futures_locks::MutexFut<()> {
        let mut map = self.0.lock().unwrap();
        let e = map.entry(key);
        let mutex_ref = e.or_insert_with(|| AsyncMutex::new(()));
        // This one is async lock, it will return future that keeps reference on underlining mutex
        // We can't return mutex_ref itself, because
        // 这一个是异步锁定，它将返回保留对下划线互斥锁的引用的未来
        // 我们不能返回mutex_ref本身，因为
        mutex_ref.lock()
    }

    fn guard_dropped(&self, key: &T) {
        let mut map = self.0.lock().unwrap();
        // This is a bit obscure, but we have to remove mutex from map before we can try
        // to unwrap it. If unwrap fails(pending references exist), we put mutex back into map
        // Since we are holding lock on map, this remove-insert does not introduces races
        // This is ok from performance point of view too:
        // removing lock is a 'happy path': if there is no pending lock,
        // then we will be able to unwrap, and in most cases won't need to re-insert mutex
        // 这有点模糊，但我们必须先从地图中删除互斥锁，然后才能尝试打开它。 如果unwrap失败（存在待定引用），我们将mutex放回map
        //由于我们在地图上持有锁定，因此此删除插入不会引入比赛
        //从性能的角度来看，这也是可以的：
        //删除锁定是一个“快乐的路径”：如果没有挂起的锁定，那么我们将能够解包，并且在大多数情况下不需要重新插入互斥锁
        let mutex = map.remove(&key);
        // It is possible that mutex is not in map, if multiple guards dropped concurrently
        // 如果多个警卫同时丢弃，则互斥锁可能不在映射中
        if let Some(mutex) = mutex {
            if let Err(cloned_mutex) = mutex.try_unwrap() {
                map.insert(key.clone(), cloned_mutex);
            }
        }
    }

    #[cfg(test)]
    pub fn held_locks_count(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

/// LockSet represents set of acquired locks
/// In addition to keeping multiple locks, LockSet protects from deadlocks
/// LockSet::lock will fail if lock is already acquired for this lock set
///
/// LockSet is useful if you acquire multiple locks in a loop, to make sure there is no deadlock.
/// It does not help if independent parts of code acquire locks and don't use LockSet.
///
/// Ideally we would want to track locks per task
/// Unfortunately, currently tokio does not support TaskLocal's, so we have to have
/// separated object to collect locks
/// LockSet表示获取的锁定集
///除了保持多个锁之外，LockSet还可以防止死锁
///如果已为此锁定集获取锁定，则LockSet :: lock将失败
///
///如果在循环中获取多个锁，LockSet很有用，以确保没有死锁。
///如果代码的独立部分获取锁并且不使用LockSet，则无效。
///
///理想情况下，我们希望跟踪每个任务的锁定
///不幸的是，目前tokio不支持TaskLocal，所以我们必须有分离的对象来收集锁
pub struct LockSet<'a, T>(&'a MutexMap<T>, HashMap<T, MutexMapGuard<T>>)
where
    T: Clone + Eq + Hash;

impl<'a, T> LockSet<'a, T>
where
    T: Clone + Eq + Hash,
{
    fn new(mutex_map: &'a MutexMap<T>) -> LockSet<'a, T> {
        LockSet(mutex_map, HashMap::new())
    }

    /// Acquires lock for given object, fails if lock was already acquired through same lock set
    /// 获取给定对象的锁定，如果已经通过相同的锁定集获取了锁定则失败
    pub async fn lock(&mut self, t: T) -> Result<(), ()> {
        if self.1.contains_key(&t) {
            return Err(());
        }
        let guard = self.0.lock(t.clone()).await;
        self.1.insert(t, guard);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::util::mutex_map::*;
    use futures::{
        channel::mpsc, executor::block_on, stream::StreamExt, FutureExt, SinkExt, TryFutureExt,
    };
    use std::{mem, sync::Arc, thread, time::Duration};
    use tokio::runtime;

    #[test()]
    pub fn test_mutex_map() {
        let map = Arc::new(MutexMap::new());
        let (sender, recv) = mpsc::unbounded();
        let guard_1 = block_on(lock_1(map.clone()));
        let mut rt1 = runtime::Builder::new().core_threads(1).build().unwrap();
        let mut rt2 = runtime::Builder::new().core_threads(1).build().unwrap();
        rt1.spawn(
            task2(map.clone(), sender.clone())
                .boxed()
                .unit_error()
                .compat(),
        );
        rt2.spawn(
            task1(map.clone(), sender.clone(), guard_1)
                .boxed()
                .unit_error()
                .compat(),
        );
        // Need to drop our reference to sender
        mem::drop(sender);
        let events: Vec<&'static str> = block_on(recv.collect());
        assert_eq!(
            vec!["Task 1 Locked 2", "Task 2 Locked 1", "Task 2 Locked 2",],
            events
        );
        // wait for runtime to shutdown so guard Drop releases memory in map
        // 等待运行时关闭，以便Guard在map中释放内存
        block_on(rt1.shutdown_on_idle().compat()).unwrap();
        block_on(rt2.shutdown_on_idle().compat()).unwrap();
        // Verify that map does not hold locks in memory when they are released
        // 验证映射在释放时不会在内存中保留锁定
        assert_eq!(0, map.held_locks_count());
    }

    async fn lock_1(map: Arc<MutexMap<usize>>) -> MutexMapGuard<usize> {
        map.lock(1).await
    }

    async fn task1(
        map: Arc<MutexMap<usize>>,
        mut actions: mpsc::UnboundedSender<&'static str>,
        _guard_1: MutexMapGuard<usize>,
    ) {
        // Sleep gives chance for task2 to acquire lock and fail test if lock_map does not work
        // correctly
        // 如果lock_map无法正常工作，则睡眠为task2获取锁定和失败测试提供了机会
        thread::sleep(Duration::from_millis(1));
        map.lock(2).await;
        actions.send("Task 1 Locked 2").await.unwrap();
        // Drops _guard_1 and releases 1 so that task2 can re-acquire this lock
    }

    async fn task2(map: Arc<MutexMap<usize>>, mut actions: mpsc::UnboundedSender<&'static str>) {
        map.lock(1).await;
        actions.send("Task 2 Locked 1").await.unwrap();
        map.lock(2).await;
        actions.send("Task 2 Locked 2").await.unwrap();
    }
}
