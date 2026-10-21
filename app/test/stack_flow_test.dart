// The UX review's flow on desktop: page, database, row, sub-page, link.
// Each lands on top at full width; see stack_flow.dart.

import 'package:flutter_test/flutter_test.dart';

import 'db_harness.dart';
import 'stack_flow.dart';

void main() {
  setUpAll(initCore);
  dbTest('page, database, row, sub-page and link each land on top', (
    tester,
  ) async {
    await pumpDb(tester, mobile: false);
    await runStackFlow(tester, (_) async {});
  });
}
