# Tester Contract — GitHub #404: failed-password alerts in the native Android app (PR 2 of #357)

Branch `feat/android-alerts`; spec `AI/architect_spec_android_alerts.md`; plan self-check
`AI/plan_evaluator_report.md` APPROVED. Matrix prefix `ANA` (rows ANA18–ANA21).

All tests below are **new**. No existing test method or assertion was modified: the two Kotlin
test files gain new imports and new `@Test` methods appended at the end; the invariant file gains
new constants, two helper functions, an add-only block inside `test_ana_s8_kotlin_hygiene` (placed
before its existing `Origin` needle, every existing assertion unchanged) and a new test function.

## JVM unit tests (run only by the CI job `Android (debug build)`; no JDK/SDK on the host)

| Test | Matrix | Red evidence |
|---|---|---|
| `ApiJsonTest.alertsActiveViewIsParsedExactly` | ANA18 | does not compile before the developer phase: `AlertsView`, `AlertRecord`, `ApiJson.parseAlerts` are unresolved |
| `ApiJsonTest.alertsEveryStateAndEmptyView` | ANA18 | same |
| `ApiJsonTest.alertsUnknownEnumValuesAreRefused` | ANA18 | same |
| `ApiJsonTest.alertsMalformedValuesAreRefused` | ANA18 | same |
| `ApiJsonTest.alertsOversizedInputIsRefused` | ANA18 | same (`ApiJson.MAX_ALERT_HISTORY` unresolved) |
| `SoosApiTest.alertsIsAPlainGet` | ANA19 | does not compile: `SoosApi.alerts` unresolved |
| `SoosApiTest.ackCarriesExactlyTheSnapshotHeadersAndNoBody` | ANA19 | does not compile: `SoosApi.ackAlerts`, `SoosApi.ACTION_ALERTS_ACK` unresolved |
| `SoosApiTest.alertsAndAckSendTheSessionCookie` | ANA19 | same |
| `SoosApiTest.ackRefusalsAreReportedWithTheirFixedToken` | ANA19 | same |
| `SoosApiTest.ackWithAnInvalidSnapshotSendsNothing` | ANA19 | same |
| `SoosApiTest.alertsWithAMalformedBodyAreUnreachable` | ANA19 | same |

What they pin: the exact view (every member, `null` handling, unknown extra members ignored),
every state, reason, coverage, source, account and kind value; unknown enum values, wrong JSON
types, negative, fractional and out-of-range numbers, malformed epochs, non-array or invalid
history refused; exactly 32 history entries accepted and 33 refused; a 16384-character body
accepted and 16385 refused. For the client: `GET /api/alerts` without `Origin`, action header or
body; `POST /api/alerts/ack` with exactly `Accept`, `Origin`, `X-Soos-Action: alerts-ack`, one
`X-Soos-Alerts-Epoch`, one `X-Soos-Alerts-Through` (in that order, plus `Cookie` when signed in),
no `Content-Type` and no body; the session cookie sent on both; refusals mapped to their fixed
token; a `200` without a view is `Unreachable`; an invalid epoch or a negative `through` sends
nothing.

## Invariant tests — `tests/invariants/src/android_native_contract.rs`

| Test | Matrix | Red evidence (`cargo test -p soos-invariants --lib android_native_contract`) |
|---|---|---|
| `test_ana_s8_kotlin_hygiene` (add-only block) | ANA20 | FAILED: `ACTION_ALERTS_ACK not found` |
| `test_ana_s14_alerts_contract` (new) | ANA20 | FAILED: `§2j must mention GET /api/alerts` |

The 12 other tests of the module passed unchanged in the same run.

Green-phase note: the forbidden needle `Text(result` of the new `test_ana_s14_alerts_contract`
also matched the existing, compliant `refusalText(result.status, result.result)` calls of
`SoosApp.kt`. Before the test was ever green, it was refined to reject only a bare `Text(result..)`
call, one not preceded by an identifier character. No pre-existing test or assertion was changed.
