use super::*;

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn median_min_and_tukey() {
    assert_eq!(median(&[]), None);
    assert_eq!(median(&[3.0]), Some(3.0));
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
    assert_eq!(median(&[5.0, 1.0, 3.0]), Some(3.0));
    assert_eq!(min(&[5.0, 1.0, 3.0]), Some(1.0));
    assert_eq!(tukey_outliers(&[1.00, 1.02, 0.99, 1.01, 3.5]), vec![4]);
    assert!(tukey_outliers(&[1.0, 1.0, 1.0, 1.0]).is_empty());
    assert!(tukey_outliers(&[1.0, 9.0, 1.0]).is_empty());
}

#[test]
fn t_distribution_matches_tables() {
    assert!(close(t_quantile(0.975, 1.0), 12.706, 0.01));
    assert!(close(t_quantile(0.975, 4.0), 2.776, 0.002));
    assert!(close(t_quantile(0.975, 8.0), 2.306, 0.002));
    assert!(close(t_quantile(0.975, 30.0), 2.042, 0.002));
    assert!(close(t_quantile(0.975, 1000.0), 1.962, 0.002));
    assert!(close(t_cdf(0.0, 5.0), 0.5, 1e-12));
    assert!(close(t_cdf(2.0, 5.0) + t_cdf(-2.0, 5.0), 1.0, 1e-12));
}

#[test]
fn welch_and_verdict() {
    assert!(welch_log(&[1.0], &[1.0, 1.1]).is_none());
    assert!(welch_log(&[1.0, 0.0], &[1.0, 1.1]).is_none());

    let a = [1.00, 1.03, 0.98, 1.02, 1.01];
    let w = welch_log(&a, &a).unwrap();
    assert!(close(w.delta_pct, 0.0, 1e-9));
    assert!(w.ci_low_pct < 0.0 && w.ci_high_pct > 0.0);
    assert!(close(w.df, 8.0, 1e-6));
    assert_eq!(verdict(&w, 0.0, 8.0), Verdict::NoChange);

    let base = [1.00, 1.02, 0.99, 1.01, 1.00];
    let change: Vec<f64> = base.iter().map(|x| x * 0.8).collect();
    let w = welch_log(&base, &change).unwrap();
    assert!(close(w.delta_pct, -20.0, 1e-6));
    assert!(w.ci_high_pct < 0.0);
    assert_eq!(verdict(&w, -20.0, 8.0), Verdict::Improved);
    assert_eq!(verdict(&w, -20.0, 25.0), Verdict::Small);
    assert_eq!(
        verdict(&welch_log(&change, &base).unwrap(), 25.0, 8.0),
        Verdict::Regressed
    );

    let w = welch_log(&[1.0, 1.0], &[1.0, 1.0]).unwrap();
    assert!(close(w.delta_pct, 0.0, 1e-9));
    assert!(w.ci_low_pct.is_finite() && w.ci_high_pct.is_finite());
}
