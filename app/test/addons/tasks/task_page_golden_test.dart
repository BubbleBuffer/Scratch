@Tags(['golden'])
library;

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/tasks/api.dart';
import 'package:notes_app/core/models.dart';

import '../../addon_harness.dart';

void main() {
  setUpAll(initCore);
  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final name = '${mobile ? 'phone' : 'desktop'}-${dark ? 'dark' : 'light'}';
      appTest('task full page $name', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark, empty: true);
        final page = await core(tester, (vault) async {
          final calendars = await vault.listCalendars();
          final page = await vault.createPage(
            title: 'Prepare the autumn garden plan',
            pageType: taskType,
          );
          await saveTask(
            vault,
            page,
            TaskData(
              status: 'doing',
              date: DateTime(2026, 10, 8, 10, 30).toUtc().toIso8601String(),
              durationMin: 90,
              assignee: 'agent',
              calendar: calendars.calendars
                  .firstWhere((c) => c.name == 'Work')
                  .id,
            ),
          );
          await vault.apply(page, [
            BOp(
              op: OpKind.insert,
              id: newId(),
              kind: Kinds.paragraph,
              text: 'Review the planting notes and choose bulbs for the front border.',
              marks: const [],
              checked: false,
            ),
          ]);
          await vault.flush();
          return page;
        });
        unawaited(openTask(tester.element(find.byType(Overlay).first), page));
        await settle(tester);
        expect(find.byType(TaskNote), findsOneWidget);
        await expectLater(
          find.byType(TaskNote),
          matchesGoldenFile(
            '../../../screenshots/calendar-task-page-$name.png',
          ),
        );
      });
    }
  }
}
