import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/ui/ui.dart';

import '../../addon_harness.dart';

Future<void> pumpTaskHost(
  WidgetTester tester,
  TestVault fixture,
  Widget child, {
  bool mobile = false,
  bool dark = false,
  double? width,
}) async {
  await tester.pumpWidget(
    MediaQuery.fromView(
      view: tester.view,
      child: AppTheme(
        colors: dark ? AppColors.darkTheme : AppColors.light,
        formFactor: mobile ? FormFactor.mobile : FormFactor.desktop,
        child: AppScope(
          state: fixture.app,
          child: WidgetsApp(
            color: dark
                ? AppColors.darkTheme.bgPrimary
                : AppColors.light.bgPrimary,
            pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
              settings: settings,
              pageBuilder: (context, _, _) => builder(context),
            ),
            home: ColoredBox(
              color: dark
                  ? AppColors.darkTheme.bgPrimary
                  : AppColors.light.bgPrimary,
              child: SafeArea(
                child: Center(
                  child: SizedBox(
                    width: width ?? (mobile ? 350 : 720),
                    child: child,
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    ),
  );
  await settle(tester);
}

Finder taskField(String name) => find.descendant(
  of: find.byKey(ValueKey('task-$name')),
  matching: find.byType(EditableText),
);

Future<AgendaItem> indexedTask(
  WidgetTester tester,
  String page,
  DateTime date,
) => core(tester, (vault) async {
  await vault.flush();
  final from = DateTime(date.year, date.month, date.day);
  final to = DateTime(date.year, date.month, date.day + 1);
  final items = await vault.calendarItems(
    timedFromMs: from.millisecondsSinceEpoch,
    timedToMs: to.millisecondsSinceEpoch,
    dayFromMs: DateTime.utc(
      from.year,
      from.month,
      from.day,
    ).millisecondsSinceEpoch,
    dayToMs: DateTime.utc(to.year, to.month, to.day).millisecondsSinceEpoch,
  );
  return items.firstWhere((item) => item.page == page);
});
