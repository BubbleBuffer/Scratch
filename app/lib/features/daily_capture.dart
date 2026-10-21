import '../core/clock.dart';
import '../core/models.dart';
import '../core/vault.dart';
import '../editor/mention_menu.dart';
import '../state/agenda_store.dart';

Future<String> todaysNote(Vault vault) async {
  final day = calendarDay(clockNow());
  return (await vault.run<String?>(
    (v) => v.dailyNote(
      dayMs: dayStamp(day),
      title: dayTitle(day, year: true),
      create: true,
    ),
    writes: true,
  ))!;
}

/// Appends above the Journal Log, keeping that toggle at the end of the note.
Future<String> captureToday(
  Vault vault,
  String text, {
  required bool task,
}) async {
  final day = calendarDay(clockNow());
  final page = (await vault.run(
    (v) => v.dailyNote(
      dayMs: dayStamp(day),
      title: dayTitle(day, year: true),
      create: true,
    ),
    writes: true,
  ))!;
  final data = await vault.loadPage(page);
  final roots = data.blocks.where((b) => b.depth == 0).toList();
  final log = roots.indexWhere(
    (b) => b.kind == Kinds.toggle && b.text == 'Log',
  );
  final after = log < 0
      ? (roots.isEmpty ? null : roots.last.id)
      : (log == 0 ? null : roots[log - 1].id);
  final mention = '@${longDate(day)}';
  final content = task ? '$text $mention' : text;
  await vault.apply(page, [
    BOp(
      op: OpKind.insert,
      id: newId(),
      after: after,
      kind: task ? Kinds.todo : Kinds.paragraph,
      text: content,
      checked: false,
      marks: task
          ? [
              BMark(
                start: text.length + 1,
                end: content.length,
                kind: BMarkKind.date,
                url: isoDate(day),
              ),
            ]
          : const [],
    ),
  ]);
  await vault.flush();
  return page;
}
