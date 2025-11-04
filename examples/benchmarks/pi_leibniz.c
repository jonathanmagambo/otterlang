#include <stdio.h>
#include <time.h>

double calculate_pi(int iterations) {
    double pi = 0.0;
    int sign = 1;
    
    for (int k = 0; k < iterations; k++) {
        pi += sign / (2.0 * k + 1.0);
        sign = -sign;
    }
    
    return pi * 4.0;
}

int main() {
    int iterations = 100000000;
    
    clock_t start = clock();
    double pi = calculate_pi(iterations);
    clock_t end = clock();
    
    double time_taken = ((double)(end - start)) / CLOCKS_PER_SEC;
    
    printf("π ≈ %.10f\n", pi);
    printf("Iterations: %d\n", iterations);
    printf("Time: %.6f seconds\n", time_taken);
    
    return 0;
}

