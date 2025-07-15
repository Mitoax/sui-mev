//! # 套利缓存管理模块
//!
//! 提供套利机会的缓存和管理功能，包括：
//! - 套利项的存储和检索
//! - 基于时间的自动过期机制
//! - 重复套利机会的去重处理
//! - 优先级队列管理

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
    time::{Duration, Instant},
};

use simulator::SimulateCtx;
use sui_types::{base_types::ObjectID, digests::TransactionDigest};

use crate::types::Source;

/// 套利项结构体
/// 
/// 包含执行套利所需的所有信息，包括币种、池ID、交易摘要、
/// 模拟上下文和事件来源。
pub struct ArbItem {
    /// 涉及的币种类型
    pub coin: String,
    /// 相关的流动性池ID（可选）
    pub pool_id: Option<ObjectID>,
    /// 原始交易的摘要
    pub tx_digest: TransactionDigest,
    /// 模拟执行上下文
    pub sim_ctx: SimulateCtx,
    /// 事件来源（公共、Shio等）
    pub source: Source,
}

impl ArbItem {
    /// 创建新的套利项
    /// 
    /// # 参数
    /// * `coin` - 币种类型
    /// * `pool_id` - 流动性池ID（可选）
    /// * `entry` - 套利缓存条目
    /// 
    /// # 返回
    /// * `Self` - 套利项实例
    pub fn new(coin: String, pool_id: Option<ObjectID>, entry: ArbEntry) -> Self {
        Self {
            coin: coin.to_string(),
            pool_id,
            tx_digest: entry.digest,
            sim_ctx: entry.sim_ctx,
            source: entry.source,
        }
    }
}

/// 套利缓存条目
/// 
/// 存储在 HashMap 中的值，包含套利机会的详细信息和元数据。
pub struct ArbEntry {
    /// 交易摘要
    digest: TransactionDigest,
    /// 模拟执行上下文
    sim_ctx: SimulateCtx,
    /// 生成代数，用于标识条目的版本
    generation: u64,
    /// 过期时间
    expires_at: Instant,
    /// 事件来源
    source: Source,
}

/// 堆项结构体
/// 
/// 用于优先级队列（二叉堆）中的元素，按过期时间排序。
#[derive(Eq, PartialEq)]
struct HeapItem {
    /// 过期时间
    expires_at: Instant,
    /// 生成代数
    generation: u64,
    /// 币种类型
    coin: String,
    /// 流动性池ID（可选）
    pool_id: Option<ObjectID>,
}

impl Ord for HeapItem {
    /// 比较堆项的顺序
    /// 
    /// 实现最小堆语义，最早过期的项排在前面。
    /// 由于 BinaryHeap 默认是最大堆，所以需要反转比较结果。
    /// 
    /// # 参数
    /// * `other` - 另一个堆项
    /// 
    /// # 返回
    /// * `Ordering` - 比较结果
    fn cmp(&self, other: &Self) -> Ordering {
        // 默认 BinaryHeap 是最大堆，所以我们反转排序：
        // 我们希望最早过期的在前面，所以时间戳比较要反转
        self.expires_at
            .cmp(&other.expires_at)
            .then(self.generation.cmp(&other.generation))
            .reverse()
    }
}

impl PartialOrd for HeapItem {
    /// 部分比较实现
    /// 
    /// 直接委托给完全比较函数。
    /// 
    /// # 参数
    /// * `other` - 另一个堆项
    /// 
    /// # 返回
    /// * `Option<Ordering>` - 比较结果
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 套利缓存管理器
/// 
/// 管理套利项的结构，提供唯一性、重排序和定时过期功能。
/// 使用 HashMap 存储当前有效的套利项，使用 BinaryHeap 管理过期时间。
pub struct ArbCache {
    /// 存储币种到套利条目的映射
    map: HashMap<String, ArbEntry>,
    /// 按过期时间排序的优先级队列
    heap: BinaryHeap<HeapItem>,
    /// 生成计数器，用于标识条目版本
    generation_counter: u64,
    /// 套利项的过期时长
    expiration_duration: Duration,
}

impl ArbCache {
    /// 创建新的套利缓存
    /// 
    /// # 参数
    /// * `expiration_duration` - 套利项的过期时长
    /// 
    /// # 返回
    /// * `Self` - 套利缓存实例
    pub fn new(expiration_duration: Duration) -> Self {
        Self {
            map: HashMap::new(),
            heap: BinaryHeap::new(),
            generation_counter: 0,
            expiration_duration,
        }
    }

    /// 插入或更新套利项
    /// 
    /// 如果币种已存在，则用新的生成代数和过期时间更新它。
    /// 这确保了每个币种只有一个活跃的套利机会。
    /// 
    /// # 参数
    /// * `coin` - 币种类型
    /// * `pool_id` - 流动性池ID（可选）
    /// * `digest` - 交易摘要
    /// * `sim_ctx` - 模拟执行上下文
    /// * `source` - 事件来源
    pub fn insert(
        &mut self,
        coin: String,
        pool_id: Option<ObjectID>,
        digest: TransactionDigest,
        sim_ctx: SimulateCtx,
        source: Source,
    ) {
        let now = Instant::now();
        self.generation_counter += 1;
        let generation = self.generation_counter;
        let expires_at = now + self.expiration_duration;

        // 插入到映射表中
        self.map.insert(
            coin.clone(),
            ArbEntry {
                digest,
                sim_ctx,
                generation,
                expires_at,
                source,
            },
        );

        // 插入到优先级队列中
        self.heap.push(HeapItem {
            expires_at,
            generation,
            coin,
            pool_id,
        });
    }

    /// 根据币种获取套利项
    /// 
    /// # 参数
    /// * `coin` - 币种类型
    /// 
    /// # 返回
    /// * `Option<(TransactionDigest, SimulateCtx)>` - 交易摘要和模拟上下文（如果存在）
    #[allow(dead_code)]
    pub fn get(&self, coin: &str) -> Option<(TransactionDigest, SimulateCtx)> {
        self.map.get(coin).map(|entry| (entry.digest, entry.sim_ctx.clone()))
    }

    /// 移除过期的条目
    /// 
    /// 定期调用此函数来清理过期的套利项。
    /// 从堆中弹出条目，直到找到一个既不过时也未过期的条目。
    /// 
    /// # 返回
    /// * `Vec<String>` - 被移除的过期币种列表
    pub fn remove_expired(&mut self) -> Vec<String> {
        let mut expired_coins = Vec::new();
        let now = Instant::now();
        while let Some(top) = self.heap.peek() {
            // 如果顶部条目过时（陈旧）或过期，弹出它并在需要时从映射中移除
            if let Some(entry) = self.map.get(&top.coin) {
                if entry.generation != top.generation {
                    // 陈旧条目，只从堆中丢弃
                    self.heap.pop();
                    continue;
                }
                // 匹配的生成代数
                if entry.expires_at <= now {
                    // 确实已过期
                    expired_coins.push(top.coin.clone());
                    self.map.remove(&top.coin);
                    self.heap.pop();
                } else {
                    // 顶部条目既未过期也不陈旧，可以退出循环
                    break;
                }
            } else {
                // 币种不在映射中意味着堆中的条目陈旧
                self.heap.pop();
            }
        }
        expired_coins
    }

    /// 弹出一个套利项
    /// 
    /// 从缓存中获取并移除一个有效的、未过期的套利项。
    /// 持续弹出直到找到一个有效的、当前的、未过期的条目。
    /// 
    /// # 返回
    /// * `Option<ArbItem>` - 套利项（如果存在有效项）
    pub fn pop_one(&mut self) -> Option<ArbItem> {
        let now = Instant::now();
        // 持续弹出直到找到一个有效的、当前的、未过期的条目
        while let Some(top) = self.heap.pop() {
            if let Some(entry) = self.map.get(&top.coin) {
                if entry.generation == top.generation {
                    // 这是该币种的当前条目
                    if entry.expires_at > now {
                        // 有效且未过期，可以移除并返回
                        let entry = self.map.remove(&top.coin).unwrap();
                        return Some(ArbItem::new(top.coin, top.pool_id, entry));
                    } else {
                        // 当前但已过期，从映射中移除并继续
                        self.map.remove(&top.coin);
                    }
                } else {
                    // 陈旧条目，直接继续而不触碰映射
                    // 因为该币种存在更新的条目
                }
            } else {
                // 映射中不再有此币种，意味着它是陈旧的
                continue;
            }
        }
        // 未找到有效条目
        None
    }
}
