# 三角套利实现示例

基于当前sui-mev架构，以下是实现简单三角套利的代码示例：

## 1. 三角套利路径结构

```rust
// 在 bin/arb/src/defi/mod.rs 中添加

#[derive(Debug, Clone)]
pub struct TriangularPath {
    pub step1: Box<dyn Dex>,  // SUI -> Token A
    pub step2: Box<dyn Dex>,  // Token A -> Token B  
    pub step3: Box<dyn Dex>,  // Token B -> SUI
    pub base_token: String,   // 基础代币 (通常是SUI)
}

impl TriangularPath {
    pub fn new(step1: Box<dyn Dex>, step2: Box<dyn Dex>, step3: Box<dyn Dex>, base_token: String) -> Self {
        Self { step1, step2, step3, base_token }
    }
    
    pub fn validate(&self) -> bool {
        // 验证路径的有效性
        let step1_out = &self.step1.coin_out_type();
        let step2_in = &self.step2.coin_in_type();
        let step2_out = &self.step2.coin_out_type();
        let step3_in = &self.step3.coin_in_type();
        let step3_out = &self.step3.coin_out_type();
        
        // 检查代币类型匹配
        step1_out == step2_in && 
        step2_out == step3_in && 
        step3_out == &self.base_token &&
        // 确保不使用相同的池子
        self.step1.object_id() != self.step2.object_id() &&
        self.step2.object_id() != self.step3.object_id() &&
        self.step1.object_id() != self.step3.object_id()
    }
    
    pub fn to_path(&self) -> Path {
        Path {
            path: vec![self.step1.clone(), self.step2.clone(), self.step3.clone()]
        }
    }
}
```

## 2. 三角套利发现算法

```rust
// 在 Defi impl 中添加

impl Defi {
    /// 发现三角套利机会
    pub async fn find_triangular_arbitrage_opportunities(
        &self,
        base_token: &str,
        min_liquidity: u128,
        max_paths: usize,
    ) -> Result<Vec<TriangularPath>> {
        let mut triangular_paths = Vec::new();
        
        // 第一步: 获取所有从base_token出发的DEX
        let step1_dexes = self.dex_searcher.find_dexes(base_token, None).await?
            .into_iter()
            .filter(|dex| dex.liquidity() >= min_liquidity)
            .collect::<Vec<_>>();
        
        for dex1 in step1_dexes {
            let token_a = dex1.coin_out_type();
            
            // 跳过如果token_a就是base_token
            if token_a == base_token {
                continue;
            }
            
            // 第二步: 从Token A到其他代币
            let step2_dexes = match self.dex_searcher.find_dexes(&token_a, None).await {
                Ok(dexes) => dexes.into_iter()
                    .filter(|dex| dex.liquidity() >= min_liquidity)
                    .filter(|dex| dex.object_id() != dex1.object_id()) // 不能使用同一个池子
                    .collect::<Vec<_>>(),
                Err(_) => continue,
            };
            
            for dex2 in step2_dexes {
                let token_b = dex2.coin_out_type();
                
                // 跳过如果token_b是base_token或token_a
                if token_b == base_token || token_b == token_a {
                    continue;
                }
                
                // 第三步: 从Token B回到base_token
                let step3_dexes = match self.dex_searcher.find_dexes(&token_b, Some(base_token.to_string())).await {
                    Ok(dexes) => dexes.into_iter()
                        .filter(|dex| dex.liquidity() >= min_liquidity)
                        .filter(|dex| dex.object_id() != dex1.object_id() && dex.object_id() != dex2.object_id())
                        .collect::<Vec<_>>(),
                    Err(_) => continue,
                };
                
                for dex3 in step3_dexes {
                    let path = TriangularPath::new(
                        dex1.clone(),
                        dex2.clone(),
                        dex3.clone(),
                        base_token.to_string(),
                    );
                    
                    if path.validate() {
                        triangular_paths.push(path);
                        
                        // 限制返回的路径数量，避免过多计算
                        if triangular_paths.len() >= max_paths {
                            return Ok(triangular_paths);
                        }
                    }
                }
            }
        }
        
        Ok(triangular_paths)
    }
    
    /// 计算三角套利的预期利润
    pub async fn calculate_triangular_profit(
        &self,
        path: &TriangularPath,
        amount_in: u64,
        sender: SuiAddress,
        gas_coins: &[ObjectRef],
        sim_ctx: &SimulateCtx,
    ) -> Result<i128> {
        let trade_path = path.to_path();
        
        let trade_result = self.trader.get_trade_result(
            &trade_path,
            sender,
            amount_in,
            TradeType::Flashloan, // 使用闪电贷进行三角套利
            gas_coins.to_vec(),
            sim_ctx.clone(),
        ).await?;
        
        // 计算净利润 (输出 - 输入 - gas费用)
        let profit = trade_result.amount_out as i128 - amount_in as i128 - trade_result.gas_cost as i128;
        
        Ok(profit)
    }
}
```

## 3. 三角套利策略集成

```rust
// 在 strategy/mod.rs 中添加三角套利监控

impl ArbStrategy {
    /// 监控三角套利机会
    pub async fn monitor_triangular_opportunities(&mut self) -> Result<()> {
        let base_tokens = vec![
            SUI_COIN_TYPE.to_string(),
            "0x5d4b302506645c37ff133b98c4b50a5ae14841659738d6d733d59d0d217a93bf::coin::COIN".to_string(), // USDC
            "0xc060006111016b8a020ad5b33834984a437aaa7d3c74c18e09a95d48aceab08c::coin::COIN".to_string(), // USDT
        ];
        
        for base_token in base_tokens {
            // 发现三角套利路径
            let triangular_paths = self.defi.find_triangular_arbitrage_opportunities(
                &base_token,
                MIN_LIQUIDITY,
                20, // 最多检查20条路径
            ).await?;
            
            // 检查每条路径的盈利性
            for path in triangular_paths {
                let test_amounts = vec![
                    1_000_000_000u64,   // 1 SUI
                    5_000_000_000u64,   // 5 SUI
                    10_000_000_000u64,  // 10 SUI
                ];
                
                for amount in test_amounts {
                    match self.calculate_triangular_profit_with_cache(&path, amount).await {
                        Ok(profit) if profit > 100_000_000 => { // 利润大于0.1 SUI
                            info!(
                                "发现三角套利机会: {} -> {} -> {} -> {}, 投入: {}, 预期利润: {}",
                                base_token,
                                path.step1.coin_out_type(),
                                path.step2.coin_out_type(),
                                base_token,
                                amount as f64 / 1_000_000_000.0,
                                profit as f64 / 1_000_000_000.0
                            );
                            
                            // 执行三角套利
                            self.execute_triangular_arbitrage(path, amount, profit as u64).await?;
                            break; // 找到盈利机会就执行，不再尝试其他金额
                        }
                        _ => continue,
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// 带缓存的利润计算
    async fn calculate_triangular_profit_with_cache(
        &self,
        path: &TriangularPath,
        amount: u64,
    ) -> Result<i128> {
        let epoch = self.get_latest_epoch().await?;
        let sim_ctx = SimulateCtx::new(epoch, vec![]);
        let gas_coins = coin::get_gas_coin_refs(&self.sui, self.sender, None).await?;
        
        self.defi.calculate_triangular_profit(
            path,
            amount,
            self.sender,
            &gas_coins,
            &sim_ctx,
        ).await
    }
    
    /// 执行三角套利
    async fn execute_triangular_arbitrage(
        &mut self,
        path: TriangularPath,
        amount_in: u64,
        expected_profit: u64,
    ) -> Result<()> {
        let epoch = self.get_latest_epoch().await?;
        let sim_ctx = SimulateCtx::new(epoch, vec![]);
        let gas_coins = coin::get_gas_coin_refs(&self.sui, self.sender, None).await?;
        
        // 构建交易
        let trade_path = path.to_path();
        let tx_data = self.defi.build_final_tx_data(
            self.sender,
            amount_in,
            &trade_path,
            gas_coins,
            sim_ctx.epoch.gas_price,
            Source::Public,
        ).await?;
        
        // 提交执行
        if let Some(sender) = &self.arb_item_sender {
            let arb_item = ArbItem {
                coin_type: path.base_token.clone(),
                pool_id: None,
                tx_digest: TransactionDigest::random(), // 临时使用随机digest
                sim_ctx,
                source: Source::Public,
                amount_in,
                expected_profit,
                tx_data: Some(tx_data),
            };
            
            sender.send(arb_item).await.map_err(|e| eyre!("发送套利任务失败: {}", e))?;
        }
        
        Ok(())
    }
}
```

## 4. 配置和使用

```rust
// 在 config.rs 中添加三角套利配置

#[derive(Debug, Clone)]
pub struct TriangularArbConfig {
    pub enabled: bool,
    pub base_tokens: Vec<String>,
    pub min_profit_threshold: u64,    // 最小利润阈值 (单位: 最小代币单位)
    pub max_amount_per_trade: u64,    // 单次交易最大金额
    pub check_interval_ms: u64,       // 检查间隔 (毫秒)
    pub max_paths_per_token: usize,   // 每个代币最多检查的路径数
}

impl Default for TriangularArbConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_tokens: vec![
                SUI_COIN_TYPE.to_string(),
                "0x5d4b302506645c37ff133b98c4b50a5ae14841659738d6d733d59d0d217a93bf::coin::COIN".to_string(), // USDC
            ],
            min_profit_threshold: 100_000_000, // 0.1 SUI
            max_amount_per_trade: 100_000_000_000, // 100 SUI
            check_interval_ms: 5000, // 5秒检查一次
            max_paths_per_token: 10,
        }
    }
}
```

## 5. 启动三角套利监控

```rust
// 在 start_bot.rs 中集成三角套利

pub async fn run(args: Args) -> Result<()> {
    // ... 现有代码 ...
    
    // 创建三角套利监控任务
    let triangular_config = TriangularArbConfig::default();
    
    if triangular_config.enabled {
        let strategy_clone = strategy.clone();
        let triangular_interval = Duration::from_millis(triangular_config.check_interval_ms);
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(triangular_interval);
            
            loop {
                interval.tick().await;
                
                if let Err(e) = strategy_clone.monitor_triangular_opportunities().await {
                    error!("三角套利监控错误: {:?}", e);
                }
            }
        });
        
        info!("三角套利监控已启动，检查间隔: {}ms", triangular_config.check_interval_ms);
    }
    
    // ... 现有代码 ...
}
```

## 6. 使用示例

```bash
# 启动带三角套利的机器人
cargo run -r --bin arb start-bot -- --private-key $SUI_PRIVATE_KEY

# 或者单独测试三角套利
cargo run -r --bin arb run --coin-type "0x2::sui::SUI" --sender $SUI_ADDRESS
```

## 注意事项

1. **风险控制**: 三角套利风险较高，建议先用小金额测试
2. **滑点影响**: 大额交易可能因滑点导致实际利润低于预期
3. **Gas费用**: 三角套利需要3次交易，Gas费用较高
4. **时间敏感**: 套利机会稍纵即逝，需要快速执行
5. **流动性要求**: 确保所有池子都有足够的流动性
6. **监控频率**: 不要过于频繁检查，避免浪费资源

这个实现基于现有的sui-mev架构，充分利用了已有的模块和接口，可以无缝集成到现有系统中。