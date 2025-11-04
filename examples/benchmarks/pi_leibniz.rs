use std::time::Instant;

fn calculate_pi(iterations: usize) -> f64 {
    let mut pi = 0.0;
    let mut sign = 1.0;
    
    for k in 0..iterations {
        pi += sign / (2.0 * k as f64 + 1.0);
        sign = -sign;
    }
    
    pi * 4.0
}

fn main() {
    let iterations = 100_000_000;
    
    let start = Instant::now();
    let pi = calculate_pi(iterations);
    let duration = start.elapsed();
    
    println!("π ≈ {:.10}", pi);
    println!("Iterations: {}", iterations);
    println!("Time: {:.6} seconds", duration.as_secs_f64());
}

