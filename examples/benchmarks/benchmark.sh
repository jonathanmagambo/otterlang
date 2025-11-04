#!/bin/bash

# Benchmark script for Leibniz formula for π
# Compares C, Rust, and OtterLang performance

set -e

echo "=== Benchmarking Leibniz Formula for π ==="
echo "Calculating π with 100,000,000 iterations"
echo ""

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

OTTER_BIN="../target/release/otter"
CACHE_DIR="$(pwd)/.otter_cache"
export OTTER_CACHE_DIR="$CACHE_DIR"

if [ ! -x "$OTTER_BIN" ]; then
    echo "Error: Otter binary not found at $OTTER_BIN. Run 'cargo build --release' first."
    exit 1
fi


# Clean previous builds
echo -e "${BLUE}Cleaning previous builds...${NC}"
rm -f pi_leibniz_c pi_leibniz_rust pi_leibniz_otter
rm -f pi_leibniz.o pi_leibniz.ll
rm -rf "$CACHE_DIR"
mkdir -p "$CACHE_DIR"

# Compile C
echo -e "${BLUE}Compiling C...${NC}"
gcc -O3 pi_leibniz.c -o pi_leibniz_c -lm
if [ ! -f pi_leibniz_c ]; then
    echo "Error: Failed to compile C"
    exit 1
fi

# Compile Rust
echo -e "${BLUE}Compiling Rust...${NC}"
rustc -O pi_leibniz.rs -o pi_leibniz_rust
if [ ! -f pi_leibniz_rust ]; then
    echo "Error: Failed to compile Rust"
    exit 1
fi

# Compile OtterLang (clean build, no cache)
echo -e "${BLUE}Compiling OtterLang...${NC}"
"$OTTER_BIN" build pi_leibniz.otter -o pi_leibniz_otter --release --no-cache
if [ ! -f pi_leibniz_otter ]; then
    echo "Error: Failed to compile OtterLang"
    exit 1
fi

echo ""
echo -e "${GREEN}Running benchmarks (5 runs each)...${NC}"
echo ""

# Benchmark C
echo -e "${YELLOW}C (gcc -O3):${NC}"
C_TIMES=()
echo "  Warm-up run (not timed)..."
./pi_leibniz_c >/dev/null 2>&1
for i in {1..5}; do
    TIME=$(/usr/bin/time -p ./pi_leibniz_c 2>&1 | grep "^real" | awk '{print $2}')
    C_TIMES+=($TIME)
    echo "  Run $i: ${TIME}s"
done

# Benchmark Rust
echo -e "${YELLOW}Rust (rustc -O):${NC}"
RUST_TIMES=()
echo "  Warm-up run (not timed)..."
./pi_leibniz_rust >/dev/null 2>&1
for i in {1..5}; do
    TIME=$(/usr/bin/time -p ./pi_leibniz_rust 2>&1 | grep "^real" | awk '{print $2}')
    RUST_TIMES+=($TIME)
    echo "  Run $i: ${TIME}s"
done

# Benchmark OtterLang
echo -e "${YELLOW}OtterLang (otter --release):${NC}"
OTTER_TIMES=()
echo "  Warm-up run (not timed)..."
./pi_leibniz_otter >/dev/null 2>&1
for i in {1..5}; do
    # Capture all output, then filter for the time line
    OUTPUT=$(/usr/bin/time -p ./pi_leibniz_otter 2>&1)
    TIME=$(echo "$OUTPUT" | grep -E "^real " | awk '{print $2}')
    if [ -z "$TIME" ]; then
        # Fallback: try to find real on any line
        TIME=$(echo "$OUTPUT" | grep "real" | tail -1 | awk '{print $NF}')
    fi
    OTTER_TIMES+=($TIME)
    echo "  Run $i: ${TIME}s"
done

# Calculate averages
C_AVG=$(printf '%s\n' "${C_TIMES[@]}" | awk '{sum+=$1; count++} END {printf "%.3f", sum/count}')
RUST_AVG=$(printf '%s\n' "${RUST_TIMES[@]}" | awk '{sum+=$1; count++} END {printf "%.3f", sum/count}')
OTTER_AVG=$(printf '%s\n' "${OTTER_TIMES[@]}" | awk '{sum+=$1; count++} END {printf "%.3f", sum/count}')

# Calculate ratios
RUST_RATIO=$(echo "scale=2; $RUST_AVG / $C_AVG" | bc)
OTTER_RATIO=$(echo "scale=2; $OTTER_AVG / $C_AVG" | bc)

echo ""
echo "=== Results ==="
echo ""
printf "%-15s %-20s %-20s %-15s\n" "Language" "Compiler" "Avg Time (5 runs)" "Relative to C"
echo "------------------------------------------------------------------------"
printf "%-15s %-20s %-20s %-15s\n" "C" "gcc -O3" "${C_AVG}s" "1.00x (baseline)"
printf "%-15s %-20s %-20s %-15s\n" "Rust" "rustc -O" "${RUST_AVG}s" "${RUST_RATIO}x"
printf "%-15s %-20s %-20s %-15s\n" "OtterLang" "otter --release" "${OTTER_AVG}s" "${OTTER_RATIO}x"
echo ""
echo "Note: These benchmarks are run with a warm-up execution and may not be 100% accurate."
echo "Results can vary based on system load, CPU throttling, and other factors."
echo ""

