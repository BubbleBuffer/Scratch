@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'harness.dart';

Future<void> _open(WidgetTester tester, {required bool mobile}) async {
  if (mobile) {
    await openMobileSettings(tester);
  } else {
    await tapAndSettle(
      tester,
      find.byKey(const ValueKey('sidebar-settings')),
    );
  }
  await tapAndSettle(
    tester,
    find.byKey(const ValueKey('settings-nav-reminders')),
  );
}

Future<void> _shot(String name) => expectLater(
  find.byType(Directionality).first,
  matchesGoldenFile('../screenshots/$name.png'),
);

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';
      appTest('reminder settings $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        await _open(tester, mobile: mobile);
        await _shot('$tag-settings-reminders');
      });
    }
  }

  appTest('reminder settings mobile light permission denied', (tester) async {
    final tv = await pumpApp(tester, mobile: true);
    tv.app.reminders.permissionGranted = false;
    await _open(tester, mobile: true);
    await _shot('mobile-light-settings-reminders-denied');
  });
}
