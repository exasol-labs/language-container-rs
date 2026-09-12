//! Statistics for `compare`: medians, Welch's t-test on log times, Tukey
//! outlier flags and the two-condition verdict. Self-contained so the driver
//! carries no numeric crate.

/// Sorted copy of `xs`.
fn sorted(xs: &[f64]) -> Vec<f64> {
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

/// Median of `xs`; `None` when empty.
pub fn median(xs: &[f64]) -> Option<f64> {
    let s = sorted(xs);
    quantile_sorted(&s, 0.5)
}

/// Minimum of `xs`; `None` when empty.
pub fn min(xs: &[f64]) -> Option<f64> {
    xs.iter().copied().min_by(|a, b| a.total_cmp(b))
}

/// Linear-interpolation quantile (R type 7) of an ascending slice.
fn quantile_sorted(s: &[f64], q: f64) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let pos = q * (s.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    Some(s[lo] + (s[hi] - s[lo]) * (pos - lo as f64))
}

/// Indexes of values outside the Tukey fences `[Q1 - 1.5 IQR, Q3 + 1.5 IQR]`.
/// Flags only; callers never remove them.
pub fn tukey_outliers(xs: &[f64]) -> Vec<usize> {
    if xs.len() < 4 {
        return Vec::new();
    }
    let s = sorted(xs);
    let q1 = quantile_sorted(&s, 0.25).unwrap_or(0.0);
    let q3 = quantile_sorted(&s, 0.75).unwrap_or(0.0);
    let iqr = q3 - q1;
    let (lo, hi) = (q1 - 1.5 * iqr, q3 + 1.5 * iqr);
    xs.iter()
        .enumerate()
        .filter(|&(_, &x)| x < lo || x > hi)
        .map(|(i, _)| i)
        .collect()
}

/// Welch's t-test of `change` against `base` on log times.
#[derive(Debug, Clone, PartialEq)]
pub struct Welch {
    /// Point estimate of the change/base ratio minus one, in percent
    /// (from the difference of mean log times).
    pub delta_pct: f64,
    /// 95 percent confidence interval on `delta_pct`.
    pub ci_low_pct: f64,
    pub ci_high_pct: f64,
    /// Welch–Satterthwaite degrees of freedom.
    pub df: f64,
}

fn mean_var(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, var)
}

/// `None` when either side has fewer than two samples or a non-positive time.
pub fn welch_log(base: &[f64], change: &[f64]) -> Option<Welch> {
    if base.len() < 2 || change.len() < 2 {
        return None;
    }
    if base.iter().chain(change).any(|&x| x.is_nan() || x <= 0.0) {
        return None;
    }
    let lb: Vec<f64> = base.iter().map(|x| x.ln()).collect();
    let lc: Vec<f64> = change.iter().map(|x| x.ln()).collect();
    let (mb, vb) = mean_var(&lb);
    let (mc, vc) = mean_var(&lc);
    let (nb, nc) = (lb.len() as f64, lc.len() as f64);
    let (sb, sc) = (vb / nb, vc / nc);
    let se = (sb + sc).sqrt().max(1e-12);
    let df = if sb + sc == 0.0 {
        nb + nc - 2.0
    } else {
        (sb + sc).powi(2) / (sb.powi(2) / (nb - 1.0) + sc.powi(2) / (nc - 1.0))
    }
    .max(1.0);
    let diff = mc - mb;
    let half = t_quantile(0.975, df) * se;
    let pct = |d: f64| (d.exp() - 1.0) * 100.0;
    Some(Welch {
        delta_pct: pct(diff),
        ci_low_pct: pct(diff - half),
        ci_high_pct: pct(diff + half),
        df,
    })
}

/// Outcome of one cell comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Improved,
    Regressed,
    Small,
    NoChange,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Improved => "improved",
            Verdict::Regressed => "regressed",
            Verdict::Small => "small",
            Verdict::NoChange => "no change",
        }
    }
}

/// `improved`/`regressed` need the interval to exclude zero and the median
/// delta outside the practical band; interval excluding zero inside the band
/// is `small`; an interval crossing zero is `no change`.
pub fn verdict(w: &Welch, median_delta_pct: f64, band_pct: f64) -> Verdict {
    let excludes_zero = w.ci_low_pct > 0.0 || w.ci_high_pct < 0.0;
    if !excludes_zero {
        return Verdict::NoChange;
    }
    if median_delta_pct.abs() <= band_pct {
        return Verdict::Small;
    }
    if median_delta_pct < 0.0 {
        Verdict::Improved
    } else {
        Verdict::Regressed
    }
}

/// Two-sided `p` quantile of Student's t with `df` degrees of freedom,
/// found by bisection on [`t_cdf`].
pub fn t_quantile(p: f64, df: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 1000.0f64);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if t_cdf(mid, df) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// CDF of Student's t via the regularized incomplete beta function.
pub fn t_cdf(t: f64, df: f64) -> f64 {
    let x = df / (df + t * t);
    let tail = 0.5 * beta_inc(0.5 * df, 0.5, x);
    if t >= 0.0 { 1.0 - tail } else { tail }
}

/// Lanczos approximation of `ln Γ(x)` for `x > 0`.
fn ln_gamma(x: f64) -> f64 {
    const G: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Regularized incomplete beta `I_x(a, b)` by Lentz's continued fraction.
fn beta_inc(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * betacf(a, b, x) / a
    } else {
        1.0 - front * betacf(b, a, 1.0 - x) / b
    }
}

fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    const EPS: f64 = 1e-14;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..300 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            break;
        }
    }
    h
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
