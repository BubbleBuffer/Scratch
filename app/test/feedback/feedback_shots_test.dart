// Golden shots of the "Send feedback" sheet: the desktop dialog and the phone
// bottom sheet, light and dark, with a typed report and the screenshot
// attached, plus the diagnostics expanded. Real fonts, as in golden_test.dart.
// Regenerate with: flutter test --update-goldens test/feedback/feedback_shots_test.dart

@Tags(['golden'])
library;

import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:package_info_plus/package_info_plus.dart';

import '../harness.dart';

const _typed = 'The sidebar froze after I opened the weekly review page.';

/// Fixed package facts; the device facts come from the Linux plugin.
void _fakePlatformInfo() {
  PackageInfo.setMockInitialValues(
    appName: 'Notes',
    packageName: 'notes.test',
    version: '0.1.0',
    buildNumber: '1',
    buildSignature: '',
  );
  DeviceInfoPlusLinuxPlugin.registerWith();
}

Finder get _sheetText => find.text('What happened?');

/// Ctrl+Shift+F, the desktop shortcut for the feedback sheet.
Future<void> _pressShortcut(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyF);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
}

/// Waits until the sheet is up: the screenshot and diagnostics are async.
Future<void> _waitForSheet(WidgetTester tester) async {
  await settleUntil(tester, () => _sheetText.evaluate().isNotEmpty);
  expect(_sheetText, findsOneWidget);
}

Future<void> _typeReport(WidgetTester tester) async {
  await tester.enterText(find.byKey(const ValueKey('feedback-text')), _typed);
  await settle(tester);
}

Future<void> _openDesktop(WidgetTester tester, {required bool dark}) async {
  _fakePlatformInfo();
  await pumpApp(tester, mobile: false, dark: dark);
  await _pressShortcut(tester);
  await _waitForSheet(tester);
  await _typeReport(tester);
}

Future<void> _openPhone(WidgetTester tester, {required bool dark}) async {
  _fakePlatformInfo();
  await pumpApp(tester, mobile: true, dark: dark);
  await openMobileSettings(tester);
  await tapAndSettle(
    tester,
    find.byKey(const ValueKey('settings-nav-general')),
  );
  await tapAndSettle(tester, find.byKey(const ValueKey('send-feedback')));
  await _waitForSheet(tester);
  await _typeReport(tester);
}

Future<void> _expandDiagnostics(WidgetTester tester) async {
  await tapAndSettle(
    tester,
    find.byKey(const ValueKey('feedback-diagnostics-toggle')),
  );
  expect(find.byKey(const ValueKey('feedback-diagnostics')), findsOneWidget);
}

/// Compares the whole app, with the sheet on top, to app/screenshots/feedback/.
Future<void> _shot(String name) => expectLater(
  find.byType(Directionality).first,
  matchesGoldenFile('../../screenshots/feedback/$name.png'),
);

void main() {
  setUpAll(initCore);

  appTest('feedback desktop light', (tester) async {
    await _openDesktop(tester, dark: false);
    await _shot('feedback-desktop-light');
  });

  appTest('feedback desktop dark', (tester) async {
    await _openDesktop(tester, dark: true);
    await _shot('feedback-desktop-dark');
  });

  appTest('feedback phone light', (tester) async {
    await _openPhone(tester, dark: false);
    await _shot('feedback-phone-light');
  });

  appTest('feedback phone dark', (tester) async {
    await _openPhone(tester, dark: true);
    await _shot('feedback-phone-dark');
  });

  appTest('feedback desktop light, diagnostics open', (tester) async {
    await _openDesktop(tester, dark: false);
    await _expandDiagnostics(tester);
    await _shot('feedback-desktop-light-diagnostics');
  });

  appTest('feedback phone dark, diagnostics open', (tester) async {
    await _openPhone(tester, dark: true);
    await _expandDiagnostics(tester);
    await _shot('feedback-phone-dark-diagnostics');
  });
}
