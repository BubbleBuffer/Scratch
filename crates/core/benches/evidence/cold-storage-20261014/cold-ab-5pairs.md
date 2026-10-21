# Cold-history A/B: five completed pairs

Status: **complete**. Generated 2026-10-14T01:26:10Z.

- Source `pairs`: pairs 1, 2, 3, 4, 5

Pair order: odd pairs run master then branch, even pairs branch then master. New pairs 3-5 run the history phase for both sides, then the sync phase for both sides.

## Setup

- History run: 60 normal samples and 1 big-page sample, 10,000 total pages including one 2,000-block page.
- Sync run: `sync_bench <work> 10000 2000` on a copy of the fixture as the hub; HTTP full sync into an empty vault.
- master fixture state (identical in all five pairs): hot 10,001 files / 309,976,276 bytes; cold 0 files / 0 bytes; dictionary 0 file / 0 bytes.
- branch fixture state (identical in all five pairs): hot 10,001 files / 35,882,765 bytes; cold 10,001 files / 126,777,760 bytes; dictionary 1 file / 8,192 bytes.

Binaries (pinned default, non-instrumented):

| Binary | sha256 | bytes | mtime (UTC) |
| --- | --- | --- | --- |
| master-bench | 3c7fe3a185c5c4811709149b3eb63c88d8c7b3275f20382e4678c6b362128151 | 9,341,784 | 2026-10-13T23:02:13Z |
| branch-bench | 5dc1e15750f86e6e73c54f31e4dca2ef7e220de5d921f05c220cfb9a9ee171e8 | 10,465,848 | 2026-10-14T00:44:54Z |
| master-sync | a0be26c43d39d0bc26050f140f8314ee230d7dea23aa44d82b188b00bd67808e | 12,071,096 | 2026-10-13T23:02:13Z |
| branch-sync | f154395a43eeb8a9b1bac30dbf547a255bfeea935cdf8d5d1b6a6478510ab3fb | 12,244,696 | 2026-10-14T00:44:54Z |

## Normal pages (ms): median over pairs of each run's reported p50

| Metric | Unit | Master median | Branch median | Branch / master | Master range | Branch range |
| --- | --- | --- | --- | --- | --- | --- |
| page open cold (fresh vault) | ms | 0.768 | 0.714 | 0.930 | 0.691 - 1.436 | 0.653 - 1.295 |
| page open warm (repeat load) | ms | 0.016 | 0.024 | 1.500 | 0.014 - 0.023 | 0.013 - 0.025 |
| history list first (fresh vault) | ms | 0.755 | 2.370 | 3.139 | 0.540 - 0.950 | 1.417 - 2.791 |
| history list repeat | ms | 0.164 | 0.154 | 0.939 | 0.101 - 0.193 | 0.078 - 0.166 |
| old version first (fresh vault) | ms | 2.950 | 4.702 | 1.594 | 2.507 - 4.571 | 2.490 - 5.028 |
| old version repeat | ms | 2.415 | 3.336 | 1.381 | 2.019 - 3.710 | 1.658 - 3.467 |
| edit first (fresh vault, cold page) | ms | 0.562 | 0.581 | 1.034 | 0.484 - 0.666 | 0.367 - 0.625 |
| edit second (same vault) | ms | 0.095 | 0.099 | 1.042 | 0.085 - 0.112 | 0.071 - 0.114 |

## Big page, 2,000 blocks (ms): median over pairs of each run's reported p50

| Metric | Unit | Master median | Branch median | Branch / master | Master range | Branch range |
| --- | --- | --- | --- | --- | --- | --- |
| page open cold (fresh vault), big page | ms | 12.880 | 17.903 | 1.390 | 9.379 - 23.173 | 9.333 - 20.231 |
| page open warm (repeat load), big page | ms | 5.791 | 7.512 | 1.297 | 4.496 - 7.276 | 3.848 - 8.174 |
| history list first (fresh vault), big page | ms | 16.297 | 108.122 | 6.634 | 13.251 - 28.066 | 54.161 - 119.073 |
| history list repeat, big page | ms | 2.824 | 3.854 | 1.365 | 2.238 - 5.185 | 1.846 - 4.179 |
| old version first (fresh vault), big page | ms | 1,449.461 | 634.823 | 0.438 | 804.528 - 1,773.950 | 360.547 - 1,213.746 |
| old version repeat, big page | ms | 1,493.495 | 547.910 | 0.367 | 825.931 - 1,789.310 | 305.022 - 1,028.646 |
| edit first (fresh vault, cold page), big page | ms | 11.392 | 9.301 | 0.816 | 8.909 - 14.709 | 7.690 - 20.673 |
| edit second (same vault), big page | ms | 0.122 | 0.110 | 0.902 | 0.103 - 0.192 | 0.085 - 0.186 |

## Full export and HTTP sync: totals per run, median over pairs

| Metric | Unit | Master median | Branch median | Branch / master | Master range | Branch range |
| --- | --- | --- | --- | --- | --- | --- |
| full export time (ms) | ms | 7,121.401 | 28,704.095 | 4.031 | 5,634.552 - 7,481.094 | 17,108.101 - 31,306.767 |
| full export bytes | bytes | 309,967,530 | 309,797,149 | 0.999 | 309,967,530 - 309,967,530 | 309,797,149 - 309,797,149 |
| HTTP full sync time (s) | s | 45.56 | 58.13 | 1.276 | 27.56 - 62.39 | 55.46 - 69.07 |
| HTTP bytes down (MB, bench prints 0.1 MB) | MB | 182.700 | 182.700 | 1.000 | 182.700 - 182.700 | 182.700 - 182.700 |
| HTTP bytes up (MB, bench prints 0.01 MB) | MB | 3.680 | 3.690 | 1.003 | 3.680 - 3.680 | 3.690 - 3.690 |

## Raw pairs (not medians)

Normal pages (ms)

| Metric | Side | P1 | P2 | P3 | P4 | P5 |
| --- | --- | --- | --- | --- | --- | --- |
| page open cold (fresh vault) | master | 0.732 | 0.768 | 0.691 | 1.119 | 1.436 |
|  | branch | 0.653 | 0.714 | 0.688 | 1.295 | 1.163 |
|  | branch / master | 0.892 | 0.930 | 0.996 | 1.157 | 0.810 |
| page open warm (repeat load) | master | 0.015 | 0.023 | 0.014 | 0.016 | 0.016 |
|  | branch | 0.025 | 0.013 | 0.016 | 0.024 | 0.025 |
|  | branch / master | 1.667 | 0.565 | 1.143 | 1.500 | 1.562 |
| history list first (fresh vault) | master | 0.652 | 0.755 | 0.540 | 0.913 | 0.950 |
|  | branch | 2.370 | 1.417 | 2.109 | 2.791 | 2.648 |
|  | branch / master | 3.635 | 1.877 | 3.906 | 3.057 | 2.787 |
| history list repeat | master | 0.116 | 0.164 | 0.101 | 0.173 | 0.193 |
|  | branch | 0.154 | 0.078 | 0.149 | 0.155 | 0.166 |
|  | branch / master | 1.328 | 0.476 | 1.475 | 0.896 | 0.860 |
| old version first (fresh vault) | master | 2.820 | 2.950 | 2.507 | 4.336 | 4.571 |
|  | branch | 4.123 | 2.490 | 4.873 | 4.702 | 5.028 |
|  | branch / master | 1.462 | 0.844 | 1.944 | 1.084 | 1.100 |
| old version repeat | master | 2.298 | 2.415 | 2.019 | 3.710 | 3.684 |
|  | branch | 2.805 | 1.658 | 3.465 | 3.336 | 3.467 |
|  | branch / master | 1.221 | 0.687 | 1.716 | 0.899 | 0.941 |
| edit first (fresh vault, cold page) | master | 0.562 | 0.484 | 0.666 | 0.654 | 0.536 |
|  | branch | 0.550 | 0.367 | 0.581 | 0.625 | 0.583 |
|  | branch / master | 0.979 | 0.758 | 0.872 | 0.956 | 1.088 |
| edit second (same vault) | master | 0.095 | 0.092 | 0.112 | 0.108 | 0.085 |
|  | branch | 0.095 | 0.071 | 0.101 | 0.114 | 0.099 |
|  | branch / master | 1.000 | 0.772 | 0.902 | 1.056 | 1.165 |

Big page (ms)

| Metric | Side | P1 | P2 | P3 | P4 | P5 |
| --- | --- | --- | --- | --- | --- | --- |
| page open cold (fresh vault), big page | master | 13.848 | 12.880 | 9.379 | 10.880 | 23.173 |
|  | branch | 20.231 | 9.333 | 17.903 | 12.001 | 20.024 |
|  | branch / master | 1.461 | 0.725 | 1.909 | 1.103 | 0.864 |
| page open warm (repeat load), big page | master | 4.844 | 6.533 | 4.496 | 7.276 | 5.791 |
|  | branch | 7.512 | 3.848 | 5.680 | 8.096 | 8.174 |
|  | branch / master | 1.551 | 0.589 | 1.263 | 1.113 | 1.412 |
| history list first (fresh vault), big page | master | 16.297 | 14.523 | 13.251 | 28.066 | 24.818 |
|  | branch | 98.740 | 54.161 | 119.073 | 108.122 | 111.194 |
|  | branch / master | 6.059 | 3.729 | 8.986 | 3.852 | 4.480 |
| history list repeat, big page | master | 2.824 | 2.621 | 2.238 | 4.770 | 5.185 |
|  | branch | 3.423 | 1.846 | 4.179 | 3.854 | 3.963 |
|  | branch / master | 1.212 | 0.704 | 1.867 | 0.808 | 0.764 |
| old version first (fresh vault), big page | master | 976.536 | 804.528 | 1,449.461 | 1,526.604 | 1,773.950 |
|  | branch | 634.823 | 360.547 | 768.530 | 601.017 | 1,213.746 |
|  | branch / master | 0.650 | 0.448 | 0.530 | 0.394 | 0.684 |
| old version repeat, big page | master | 943.030 | 825.931 | 1,493.495 | 1,789.310 | 1,752.227 |
|  | branch | 547.910 | 305.022 | 672.771 | 534.348 | 1,028.646 |
|  | branch / master | 0.581 | 0.369 | 0.450 | 0.299 | 0.587 |
| edit first (fresh vault, cold page), big page | master | 12.503 | 8.909 | 14.709 | 10.200 | 11.392 |
|  | branch | 8.165 | 7.690 | 15.557 | 20.673 | 9.301 |
|  | branch / master | 0.653 | 0.863 | 1.058 | 2.027 | 0.816 |
| edit second (same vault), big page | master | 0.136 | 0.103 | 0.192 | 0.113 | 0.122 |
|  | branch | 0.098 | 0.085 | 0.186 | 0.178 | 0.110 |
|  | branch / master | 0.721 | 0.825 | 0.969 | 1.575 | 0.902 |

Full export and HTTP sync

| Metric | Side | P1 | P2 | P3 | P4 | P5 |
| --- | --- | --- | --- | --- | --- | --- |
| full export time (ms) | master | 5,634.552 | 5,715.044 | 7,121.401 | 7,481.094 | 7,338.573 |
|  | branch | 22,044.677 | 17,108.101 | 28,704.095 | 30,057.231 | 31,306.767 |
|  | branch / master | 3.912 | 2.994 | 4.031 | 4.018 | 4.266 |
| full export bytes | master | 309,967,530 | 309,967,530 | 309,967,530 | 309,967,530 | 309,967,530 |
|  | branch | 309,797,149 | 309,797,149 | 309,797,149 | 309,797,149 | 309,797,149 |
|  | branch / master | 0.999 | 0.999 | 0.999 | 0.999 | 0.999 |
| HTTP full sync time (s) | master | 40.07 | 27.56 | 45.56 | 62.39 | 48.77 |
|  | branch | 56.11 | 68.99 | 58.13 | 69.07 | 55.46 |
|  | branch / master | 1.400 | 2.503 | 1.276 | 1.107 | 1.137 |
| HTTP bytes down (MB, bench prints 0.1 MB) | master | 182.700 | 182.700 | 182.700 | 182.700 | 182.700 |
|  | branch | 182.700 | 182.700 | 182.700 | 182.700 | 182.700 |
|  | branch / master | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 |
| HTTP bytes up (MB, bench prints 0.01 MB) | master | 3.680 | 3.680 | 3.680 | 3.680 | 3.680 |
|  | branch | 3.690 | 3.690 | 3.690 | 3.690 | 3.690 |
|  | branch / master | 1.003 | 1.003 | 1.003 | 1.003 | 1.003 |

## Load (1 / 5 / 15 minute, raw readings)

Each row is one side-run. Start and end readings are taken around the run. Source shows whether the reading came from `ssh` to the home server or from this runner's /proc/loadavg.

| Pair | Side | Phase | Run | Start 1/5/15 | Start source | End 1/5/15 | End source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | master | history | aged-master-1 | 52.26 / 43.55 / 30.98 | fallback /proc/loadavg on this runner (ssh disabled) | 55.82 / 45.90 / 32.44 | fallback /proc/loadavg on this runner (ssh disabled) |
| 1 | master | sync | aged-master-sync-1 | 46.04 / 45.70 / 33.53 | fallback /proc/loadavg on this runner (ssh disabled) | 35.14 / 42.26 / 33.32 | fallback /proc/loadavg on this runner (ssh disabled) |
| 1 | branch | history | aged-branch-1 | 57.28 / 46.37 / 32.67 | fallback /proc/loadavg on this runner (ssh disabled) | 46.04 / 45.70 / 33.53 | fallback /proc/loadavg on this runner (ssh disabled) |
| 1 | branch | sync | aged-branch-sync-1 | 35.14 / 42.26 / 33.32 | fallback /proc/loadavg on this runner (ssh disabled) | 13.09 / 32.57 / 30.97 | fallback /proc/loadavg on this runner (ssh disabled) |
| 2 | master | history | aged-master-2 | 5.53 / 26.26 / 28.89 | fallback /proc/loadavg on this runner (ssh disabled) | 5.06 / 22.94 / 27.63 | fallback /proc/loadavg on this runner (ssh disabled) |
| 2 | master | sync | aged-master-sync-2 | 19.94 / 23.33 / 27.14 | fallback /proc/loadavg on this runner (ssh disabled) | 9.65 / 19.70 / 25.63 | fallback /proc/loadavg on this runner (ssh disabled) |
| 2 | branch | history | aged-branch-2 | 10.25 / 30.68 / 30.38 | fallback /proc/loadavg on this runner (ssh disabled) | 5.53 / 26.26 / 28.89 | fallback /proc/loadavg on this runner (ssh disabled) |
| 2 | branch | sync | aged-branch-sync-2 | 5.21 / 22.68 / 27.52 | fallback /proc/loadavg on this runner (ssh disabled) | 21.07 / 23.60 / 27.25 | fallback /proc/loadavg on this runner (ssh disabled) |
| 3 | master | history | aged-master-3 | 8.79 / 19.19 / 25.40 | fallback /proc/loadavg on this runner (ssh disabled) | 9.36 / 17.42 / 24.43 | fallback /proc/loadavg on this runner (ssh disabled) |
| 3 | master | sync | aged-master-sync-3 | 14.79 / 16.93 / 23.53 | fallback /proc/loadavg on this runner (ssh disabled) | 16.30 / 16.81 / 22.87 | fallback /proc/loadavg on this runner (ssh disabled) |
| 3 | branch | history | aged-branch-3 | 10.13 / 17.45 / 24.40 | fallback /proc/loadavg on this runner (ssh disabled) | 14.77 / 16.96 / 23.57 | fallback /proc/loadavg on this runner (ssh disabled) |
| 3 | branch | sync | aged-branch-sync-3 | 16.27 / 16.80 / 22.83 | fallback /proc/loadavg on this runner (ssh disabled) | 19.67 / 17.69 / 22.38 | fallback /proc/loadavg on this runner (ssh disabled) |
| 4 | master | history | aged-master-4 | 25.83 / 19.93 / 22.65 | fallback /proc/loadavg on this runner (ssh disabled) | 27.01 / 21.99 / 23.19 | fallback /proc/loadavg on this runner (ssh disabled) |
| 4 | master | sync | aged-master-sync-4 | 22.33 / 21.96 / 23.00 | fallback /proc/loadavg on this runner (ssh disabled) | 26.19 / 23.74 / 23.54 | fallback /proc/loadavg on this runner (ssh disabled) |
| 4 | branch | history | aged-branch-4 | 19.71 / 17.77 / 22.35 | fallback /proc/loadavg on this runner (ssh disabled) | 25.04 / 19.68 / 22.59 | fallback /proc/loadavg on this runner (ssh disabled) |
| 4 | branch | sync | aged-branch-sync-4 | 27.17 / 22.11 / 23.22 | fallback /proc/loadavg on this runner (ssh disabled) | 22.33 / 21.96 / 23.00 | fallback /proc/loadavg on this runner (ssh disabled) |
| 5 | master | history | aged-master-5 | 21.78 / 22.96 / 23.28 | fallback /proc/loadavg on this runner (ssh disabled) | 26.06 / 23.78 / 23.53 | fallback /proc/loadavg on this runner (ssh disabled) |
| 5 | master | sync | aged-master-sync-5 | 25.00 / 24.01 / 23.64 | fallback /proc/loadavg on this runner (ssh disabled) | 19.65 / 22.44 / 23.11 | fallback /proc/loadavg on this runner (ssh disabled) |
| 5 | branch | history | aged-branch-5 | 26.06 / 23.81 / 23.54 | fallback /proc/loadavg on this runner (ssh disabled) | 24.57 / 23.91 / 23.60 | fallback /proc/loadavg on this runner (ssh disabled) |
| 5 | branch | sync | aged-branch-sync-5 | 19.65 / 22.44 / 23.11 | fallback /proc/loadavg on this runner (ssh disabled) | 18.53 / 21.42 / 22.66 | fallback /proc/loadavg on this runner (ssh disabled) |

Pair boundary readings:

| Pair | Edge | 1/5/15 | Source |
| --- | --- | --- | --- |
| 1 | before | 51.09 / 43.04 / 30.68 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 1 | after | 13.09 / 32.57 / 30.97 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 2 | before | 12.11 / 31.73 / 30.71 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 2 | after | 9.65 / 19.70 / 25.63 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 3 | before | 9.12 / 19.43 / 25.51 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 3 | after | 19.86 / 17.77 / 22.38 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 4 | before | 19.86 / 17.77 / 22.38 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 4 | after | 25.79 / 23.73 / 23.54 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 5 | before | 22.54 / 23.14 / 23.35 | ssh home-server uptime, run from the Windows PC via ssh.exe |
| 5 | after | 18.33 / 21.33 / 22.62 | ssh home-server uptime, run from the Windows PC via ssh.exe |

Reading sources: 40 x fallback /proc/loadavg on this runner (ssh disabled).

## Retained failures and exclusions

Known flush-lock timeouts listed here: 0. They are owned separately; their failed attempts remain separate from the selected successful pairs.

No failed or excluded runs were found in the sources.

## Caveats

- Five pairs is the minimum. Each median is over five values, with no confidence interval, and no stability or significance verdict is claimed.
- Master and branch do not read the same on-disk state. Master reads the hot-only fixture; branch reads the fixture with cold files. The ratios compare those two states, as the v4 pairs did.
- Cold means a freshly opened vault in the bench process. OS page caches were not dropped.
- HTTP byte counts are the bench's printed MB values (0.1 MB precision for down, 0.01 MB for up). Full export bytes are exact.
- Load readings have no threshold applied. Each reading names its source.
