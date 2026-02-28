# BOJ Problem Survey (1152 + 10)

This survey captures what `nado` needs to support beyond BOJ 1000.

- Official URLs are the target problems.
- Constraint details were extracted from public writeups because BOJ pages are bot-restricted in this environment.

## Summary Matrix

| Problem | Official | Input/Constraint Snapshot | What `nado` needs |
|---|---|---|---|
| 1152 - 단어의 개수 | https://www.acmicpc.net/problem/1152 | One line sentence (`<= 1,000,000` chars), words separated by spaces, leading/trailing spaces possible | `line.string` generator + whitespace edge-case corpus + token-count comparator |
| 1546 - 평균 | https://www.acmicpc.net/problem/1546 | `N (1..1000)`, next line `N` scores (`0..100`) | Dependent generator (`N` then list length `N`), float comparator with epsilon |
| 2675 - 문자열 반복 | https://www.acmicpc.net/problem/2675 | `T`, then for each case: `R(1..8)` and string `S` (`|S| <= 20`) | Repeated testcase generator + bounded alnum string generator |
| 1157 - 단어 공부 | https://www.acmicpc.net/problem/1157 | Single alphabetic word (`|W| <= 1,000,000`, case-insensitive) | Alpha-only string generator + case-mix fuzzing |
| 11720 - 숫자의 합 | https://www.acmicpc.net/problem/11720 | `N (1..100)`, then a digit string of length `N` | Exact-length digit string generator |
| 10809 - 알파벳 찾기 | https://www.acmicpc.net/problem/10809 | Lowercase word (`|S| <= 100`) | Lowercase-only string generator + fixed-width integer vector output check |
| 2908 - 상수 | https://www.acmicpc.net/problem/2908 | Two 3-digit numbers, no trailing zero | Integer generator with digit-level constraints |
| 2577 - 숫자의 개수 | https://www.acmicpc.net/problem/2577 | 3 natural numbers (`<= 100`) then digit-frequency output (`0..9`) | Multi-line scalar input + exact 10-line output matcher |
| 2562 - 최댓값 | https://www.acmicpc.net/problem/2562 | 9 natural numbers (`< 100`) line-by-line | Fixed-count line input generator (`count = 9`) |
| 2920 - 음계 | https://www.acmicpc.net/problem/2920 | Exactly 8 integers in range `1..8` | Fixed-size vector generator (`count = 8`, bounded range) |
| 8958 - OX퀴즈 | https://www.acmicpc.net/problem/8958 | `T`, then each line is `O/X` string (`0 < len < 80`) | Repeated testcase + regex-constrained string generator (`[OX]+`) |

## Prioritized Capability Gaps (from this survey)

1. Dependent shapes (`N`/`T`-driven dynamic lengths)
2. Rich string generation (alphabet set, regex-ish classes, edge whitespace)
3. Fixed-length vector generation
4. Float-tolerant comparator mode
5. Multi-case input templates

## Constraint Sources

- 1152: https://st-lab.tistory.com/41
- 1546: https://go2gym365.tistory.com/195
- 2675: https://st-lab.tistory.com/267
- 1157: https://st-lab.tistory.com/194
- 11720: https://go2gym365.tistory.com/65
- 10809: https://freshrimpsushi.github.io/posts/3120/
- 2908: https://st-lab.tistory.com/73
- 2577: https://hellodoor.tistory.com/44
- 2562: https://go2gym365.tistory.com/69
- 2920: https://my-coding-notes.tistory.com/88
- 8958: https://my-coding-notes.tistory.com/46
