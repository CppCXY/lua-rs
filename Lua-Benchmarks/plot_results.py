#!/usr/bin/env python3
"""Generate PNG charts from runbenchmarks.ps1 / runbenchmarks.lua output.

Usage: python plot_results.py [base_name] [--output_dir DIR]
Default base_name is 'results'. Reads <base>.dat, <base>-norm.dat, <base>-speed.dat
and writes <base>.png, <base>-norm.png, <base>-speed.png.
"""

import argparse
import os
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

COLORS = ["#4C72B0", "#DD8452", "#55A868", "#C44E52", "#8172B3", "#937860", "#DA8BC3"]


def read_dat(path):
    tests, rows = [], []
    with open(path, "r", encoding="ascii") as f:
        lines = [ln.rstrip("\n").rstrip("\r") for ln in f if ln.strip()]
    binaries = lines[0].split("\t")[1:]
    for ln in lines[1:]:
        parts = ln.split("\t")
        tests.append(parts[0])
        rows.append([float(x) if x != "NaN" else None for x in parts[1:]])
    return tests, binaries, rows


def make_chart(tests, binaries, rows, outfile, ylabel, lower_is_better):
    n_tests = len(tests)
    n_bin = len(binaries)
    x = range(n_tests)
    width = 0.9 / n_bin

    fig, ax = plt.subplots(figsize=(30, 10))
    fig.patch.set_facecolor("white")

    for j, name in enumerate(binaries):
        vals = [rows[i][j] if rows[i][j] is not None else 0.0 for i in range(n_tests)]
        pos = [xi + width * (j - (n_bin - 1) / 2) for xi in x]
        ax.bar(pos, vals, width=width, label=name, color=COLORS[j % len(COLORS)])

    ax.set_xticks(list(x))
    ax.set_xticklabels(tests, rotation=30, ha="right", fontsize=14)
    ax.set_ylabel(ylabel, fontsize=22)
    ax.yaxis.grid(True, linestyle="--", alpha=0.6)
    ax.set_axisbelow(True)
    ax.tick_params(axis="y", labelsize=18)
    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)
    ax.legend(fontsize=18, loc="upper center", bbox_to_anchor=(0.5, -0.12), ncol=n_bin)

    fig.tight_layout()
    fig.savefig(outfile, dpi=120, bbox_inches="tight")
    plt.close(fig)
    print(f"Created {outfile}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base", nargs="?", default="results", help="output file base name (default: results)")
    ap.add_argument("--output_dir", default=".", help="directory to write PNGs (default: current dir)")
    args = ap.parse_args()

    base = args.base
    spec = [
        (f"{base}.dat", "Elapsed time (sec)", False),
        (f"{base}-norm.dat", "Normalized time (lower is better)", True),
        (f"{base}-speed.dat", "Speedup factor (higher is better)", True),
    ]

    for datfile, ylabel, lower_is_better in spec:
        if not os.path.isfile(datfile):
            print(f"Warning: {datfile} not found, skipping.", file=sys.stderr)
            continue
        tests, binaries, rows = read_dat(datfile)
        outfile = os.path.join(args.output_dir, os.path.splitext(os.path.basename(datfile))[0] + ".png")
        make_chart(tests, binaries, rows, outfile, ylabel, lower_is_better)


if __name__ == "__main__":
    main()
