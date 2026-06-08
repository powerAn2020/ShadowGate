//! RSSI 信号平滑与滤波算法
//!
//! 提供两种滤波策略:
//! 1. **滑动平均 (Moving Average)**: 轻量级，适合 Android 端
//! 2. **卡尔曼滤波 (Kalman Filter)**: 更精确，适合 PC 端信号处理
//!
//! 同时实现了双向不对称迟滞 (Hysteresis) 防抖逻辑防止人体遮挡误判。

/// 一维卡尔曼滤波器 (用于 RSSI 信号平滑)
///
/// 状态方程: x_k = x_{k-1} (RSSI 在短时间内假设恒定)
/// 观测方程: z_k = x_k + v_k (观测 = 真值 + 噪声)
#[derive(Debug, Clone)]
pub struct KalmanFilter {
    /// 状态估计值 (当前最优 RSSI 估计)
    state: f64,
    /// 估计误差协方差
    covariance: f64,
    /// 过程噪声协方差 (Q) - 越大表示对状态变化越敏感
    process_noise: f64,
    /// 观测噪声协方差 (R) - 越大表示对观测值越不信任
    measurement_noise: f64,
}

impl KalmanFilter {
    /// 创建新的卡尔曼滤波器
    ///
    /// # Panics
    /// 如果 process_noise 或 measurement_noise <= 0
    pub fn new(
        initial_rssi: f64,
        process_noise: f64,
        measurement_noise: f64,
    ) -> Self {
        assert!(
            process_noise > 0.0 && measurement_noise > 0.0,
            "Noise parameters must be positive"
        );
        KalmanFilter {
            state: initial_rssi,
            covariance: 1.0, // 初始不确定性
            process_noise,
            measurement_noise,
        }
    }

    /// 输入新的 RSSI 观测值，返回滤波后的估计值
    pub fn update(&mut self, measurement: f64) -> f64 {
        // 预测步骤 (Predict)
        let predicted_state = self.state;
        let predicted_covariance = self.covariance + self.process_noise;

        // 更新步骤 (Update)
        let kalman_gain = predicted_covariance / (predicted_covariance + self.measurement_noise);
        self.state = predicted_state + kalman_gain * (measurement - predicted_state);
        self.covariance = (1.0 - kalman_gain) * predicted_covariance;

        self.state
    }

    /// 获取当前估计值
    pub fn current_estimate(&self) -> f64 {
        self.state
    }

    /// 重置滤波器状态
    pub fn reset(&mut self, initial_rssi: f64) {
        self.state = initial_rssi;
        self.covariance = 1.0;
    }
}

impl Default for KalmanFilter {
    /// 默认参数适用于典型室内 BLE 环境:
    /// - process_noise = 2.0 (人体缓慢移动)
    /// - measurement_noise = 4.0 (BLE RSSI 本底噪声约 ±4dBm)
    fn default() -> Self {
        KalmanFilter::new(-70.0, 2.0, 4.0)
    }
}

/// 滑动平均滤波器 (Moving Average)
///
/// 轻量级实现，维护一个固定大小的环形缓冲区。
#[derive(Debug, Clone)]
pub struct MovingAverageFilter {
    buffer: Vec<f64>,
    window_size: usize,
    index: usize,
    count: usize, // 已填充的样本数 (小于 window_size 时用于计算正确均值)
}

impl MovingAverageFilter {
    /// 创建滑动平均滤波器
    pub fn new(window_size: usize) -> Self {
        assert!(window_size > 0, "window_size must be > 0");
        MovingAverageFilter {
            buffer: vec![0.0; window_size],
            window_size,
            index: 0,
            count: 0,
        }
    }

    /// 添加新样本，返回当前均值
    pub fn push(&mut self, value: f64) -> f64 {
        self.buffer[self.index] = value;
        self.index = (self.index + 1) % self.window_size;
        if self.count < self.window_size {
            self.count += 1;
        }

        let sum: f64 = self.buffer[..self.count].iter().sum();
        sum / self.count as f64
    }

    /// 获取当前均值 (不添加新样本)
    pub fn mean(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let sum: f64 = self.buffer[..self.count].iter().sum();
        sum / self.count as f64
    }

    /// 重置滤波器
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
        self.count = 0;
    }
}

/// 双向不对称迟滞 (Hysteresis) 状态机
///
/// 用于防止：
/// - **解锁振荡**: 设备刚好在阈值边界反复触发解锁/锁定
/// - **人体遮挡误判**: 短暂遮挡导致的 RSSI 骤降
///
/// 策略:
/// - 解锁: RSSI 需连续 `unlock_samples` 次 > unlock_threshold
/// - 锁定: RSSI 需连续 `lock_samples` 次 < lock_threshold (或持续 `lock_duration_ms` 毫秒)
#[derive(Debug, Clone)]
pub struct HysteresisDetector {
    /// 解锁阈值 (dBm)，信号强度高于此值可能触发解锁
    unlock_threshold: f64,
    /// 锁定阈值 (dBm)，信号强度低于此值可能触发锁定
    lock_threshold: f64,
    /// 解锁确认所需连续样本数
    unlock_samples_required: usize,
    /// 锁定确认所需连续样本数
    lock_samples_required: usize,
    /// 连续满足解锁条件的计数
    unlock_count: usize,
    /// 连续满足锁定条件的计数
    lock_count: usize,
}

impl HysteresisDetector {
    /// 创建迟滞检测器
    pub fn new(
        unlock_threshold: f64,
        lock_threshold: f64,
        unlock_samples_required: usize,
        lock_samples_required: usize,
    ) -> Self {
        assert!(unlock_threshold > lock_threshold,
            "unlock_threshold must be > lock_threshold to form a hysteresis buffer zone");
        HysteresisDetector {
            unlock_threshold,
            lock_threshold,
            unlock_samples_required,
            lock_samples_required,
            unlock_count: 0,
            lock_count: 0,
        }
    }

    /// 输入当前 RSSI 值，返回是否应该触发动作
    ///
    /// 返回 `HysteresisAction` 枚举:
    /// - `None`: 无动作
    /// - `Unlock`: 应解锁
    /// - `Lock`: 应锁定
    pub fn update(&mut self, rssi: f64) -> HysteresisAction {
        if rssi >= self.unlock_threshold {
            self.unlock_count += 1;
            self.lock_count = 0; // 重置锁屏计数

            if self.unlock_count >= self.unlock_samples_required {
                self.unlock_count = 0; // 防止重复触发
                return HysteresisAction::Unlock;
            }
        } else if rssi <= self.lock_threshold {
            self.lock_count += 1;
            self.unlock_count = 0; // 重置解锁计数

            if self.lock_count >= self.lock_samples_required {
                self.lock_count = 0;
                return HysteresisAction::Lock;
            }
        } else {
            // 处于缓冲区 (lock_threshold < rssi < unlock_threshold)
            // 不改变任何计数 — 保持当前状态
        }

        HysteresisAction::None
    }

    /// 重置所有状态
    pub fn reset(&mut self) {
        self.unlock_count = 0;
        self.lock_count = 0;
    }
}

/// 迟滞检测器输出动作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HysteresisAction {
    /// 无需任何动作
    None,
    /// 应执行解锁
    Unlock,
    /// 应执行锁定
    Lock,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kalman_filter_convergence() {
        let mut kf = KalmanFilter::new(-70.0, 2.0, 4.0);
        // 多次输入 -60 观测值，滤波器应逐渐收敛
        for _ in 0..20 {
            kf.update(-60.0);
        }
        let estimate = kf.current_estimate();
        // 应接近 -60.0
        assert!(estimate < -55.0 && estimate > -65.0,
            "Kalman estimate {} not converging near -60", estimate);
    }

    #[test]
    fn test_moving_average() {
        let mut ma = MovingAverageFilter::new(5);
        assert_eq!(ma.push(10.0), 10.0);
        assert_eq!(ma.push(20.0), 15.0);
        assert_eq!(ma.push(30.0), 20.0);
        assert_eq!(ma.push(40.0), 25.0);
        assert_eq!(ma.push(50.0), 30.0);
        // 窗口满了后，旧值被挤出
        assert_eq!(ma.push(60.0), 40.0); // (20+30+40+50+60)/5 = 40
    }

    #[test]
    fn test_hysteresis_unlock() {
        let mut det = HysteresisDetector::new(-60.0, -80.0, 3, 5);
        // 连续 3 次高于 -60dBm 应触发解锁
        assert_eq!(det.update(-55.0), HysteresisAction::None);
        assert_eq!(det.update(-55.0), HysteresisAction::None);
        assert_eq!(det.update(-55.0), HysteresisAction::Unlock);
    }

    #[test]
    fn test_hysteresis_lock() {
        let mut det = HysteresisDetector::new(-60.0, -80.0, 3, 5);
        // 连续 5 次低于 -80dBm 应触发锁定
        for _ in 0..4 {
            assert_eq!(det.update(-85.0), HysteresisAction::None);
        }
        assert_eq!(det.update(-85.0), HysteresisAction::Lock);
    }

    #[test]
    fn test_hysteresis_buffer_zone_no_action() {
        let mut det = HysteresisDetector::new(-60.0, -80.0, 3, 3);
        // 处于缓冲区的值不触发任何动作
        assert_eq!(det.update(-70.0), HysteresisAction::None);
        assert_eq!(det.update(-70.0), HysteresisAction::None);
    }
}
