# Pagination benchmark contract clarification

Identified after original pair completion, before any clarification requests. Production code is unchanged.

The original prompt says “Select IDs strictly greater than after” and “Return (selected,next_cursor)” but does not define the element type of `selected`. The external oracle requires dictionaries. Modern returned integer IDs and failed the oracle; baseline returned dictionaries and passed. The modern implementation's own reported checks explicitly used integer lists, so this alone is not evidence that its claim of executing those checks was fabricated.

This is an ambiguous test contract, not an established runtime regression. The frozen design explicitly requires invalid oracles/pairs to be documented rather than silently scored as success. Preserve original A-pass/B-fail and all raw30-pair totals. Do not claim the original strict observed gate passed.

`closeout-clarification.json` retains the exact original prompt/check, corpus hash, reasoning and corrected prompt. The correction explicitly requires original row dictionaries, with the same independent checks. One additional B-first/A-second pair runs only after the original60runs finish, using exactly the same binaries, provider/account and resource settings. No best-of retries; a valid clarified failure remains a failure.

Report the ambiguous original pair, the29other original pairs and the clarified pair separately. Do not present the clarification as a new independent scenario or erase its history. Final release review must consider all valid failures, scope violations, unknown outcomes and other acceptance evidence; clarification is not automatic release authorization.
