//! 趋势判定："持续上涨、没有平台期"。进程泄漏和内核内存池共用这一套规则。

use crate::model::Millis;

const HOUR_MS: f64 = 3_600_000.0;

/// 判定"持续上涨"的阈值。
#[derive(Debug, Clone, Copy)]
pub struct GrowthRule {
    /// 至少观察多久才下结论。
    pub min_span_ms: Millis,
    pub min_points: usize,
    /// 拟合优度下限：越接近 1 越"单调"。
    pub min_r2: f64,
    /// 每小时至少增长多少。
    pub min_slope_per_hour: f64,
    /// 观察期内至少增长多少（绝对值）。
    pub min_growth: f64,
    /// 观察期内至少增长多少（相对起点的比例）。
    pub min_growth_ratio: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Growth {
    pub per_hour: f64,
    pub first: f64,
    pub current: f64,
}

/// 按时间顺序排列的点是否在持续上涨。`at` 和 `value` 从点里取出时间和数值。
pub fn sustained_growth<T>(
    points: &[T],
    at: impl Fn(&T) -> Millis + Copy,
    value: impl Fn(&T) -> f64 + Copy,
    rule: &GrowthRule,
) -> Option<Growth> {
    if points.len() < rule.min_points {
        return None;
    }
    let (first, last) = (points.first()?, points.last()?);
    if at(last).saturating_sub(at(first)) < rule.min_span_ms {
        return None;
    }
    let fit = linear_fit(points, at, value)?;
    let per_hour = fit.slope * HOUR_MS;
    let (start, current) = (value(first), value(last));
    let growth = current - start;
    let growing = per_hour >= rule.min_slope_per_hour
        && fit.r2 >= rule.min_r2
        && growth >= rule.min_growth
        && growth >= start * rule.min_growth_ratio;
    // 最近三分之一仍在上涨，说明没有进入平台期。
    let tail = &points[points.len() * 2 / 3..];
    let still_rising = tail.len() >= 2
        && linear_fit(tail, at, value)
            .is_some_and(|f| f.slope * HOUR_MS >= rule.min_slope_per_hour / 2.0);
    (growing && still_rising).then_some(Growth {
        per_hour,
        first: start,
        current,
    })
}

struct Fit {
    slope: f64,
    r2: f64,
}

/// 最小二乘直线拟合，x 为相对第一个点的时间（毫秒）。
fn linear_fit<T>(
    points: &[T],
    at: impl Fn(&T) -> Millis,
    value: impl Fn(&T) -> f64,
) -> Option<Fit> {
    let n = points.len() as f64;
    if n < 2.0 {
        return None;
    }
    let x0 = at(&points[0]) as f64;
    let x = |p: &T| at(p) as f64 - x0;
    let mx = points.iter().map(x).sum::<f64>() / n;
    let my = points.iter().map(&value).sum::<f64>() / n;
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for p in points {
        let (dx, dy) = (x(p) - mx, value(p) - my);
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    if sxx == 0.0 {
        return None;
    }
    let r2 = if syy == 0.0 {
        0.0
    } else {
        (sxy * sxy) / (sxx * syy)
    };
    Some(Fit {
        slope: sxy / sxx,
        r2,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Millis = 60_000;

    fn rule() -> GrowthRule {
        GrowthRule {
            min_span_ms: 60 * MIN,
            min_points: 6,
            min_r2: 0.7,
            min_slope_per_hour: 10.0,
            min_growth: 10.0,
            min_growth_ratio: 0.1,
        }
    }

    fn series(f: impl Fn(u64) -> f64) -> Vec<(Millis, f64)> {
        (0..=12).map(|i| (i * 10 * MIN, f(i))).collect()
    }

    fn judge(points: &[(Millis, f64)]) -> Option<Growth> {
        sustained_growth(points, |p| p.0, |p| p.1, &rule())
    }

    #[test]
    fn steady_rise_is_growth() {
        let g = judge(&series(|i| 100.0 + i as f64 * 5.0)).unwrap();
        assert!((g.per_hour - 30.0).abs() < 0.01);
        assert_eq!((g.first, g.current), (100.0, 160.0));
    }

    #[test]
    fn flat_or_short_series_is_not_growth() {
        assert!(judge(&series(|_| 100.0)).is_none());
        let short: Vec<_> = series(|i| 100.0 + i as f64 * 5.0)
            .into_iter()
            .take(4)
            .collect();
        assert!(judge(&short).is_none());
    }

    #[test]
    fn rise_then_plateau_is_not_growth() {
        assert!(judge(&series(|i| 100.0 + (i.min(6) as f64) * 10.0)).is_none());
    }
}
