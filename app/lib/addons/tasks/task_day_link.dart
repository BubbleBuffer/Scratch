import 'package:flutter/foundation.dart';

import '../../core/vault.dart';

/// Adds a Journal link without undoing a successful task metadata save.
/// Day stamps use UTC midnight for the task's local calendar date.
Future<void> linkTaskToDay(
  Vault vault,
  String taskPageId,
  DateTime? date,
) async {
  if (date == null) return;
  final dayMs = DateTime.utc(
    date.year,
    date.month,
    date.day,
  ).millisecondsSinceEpoch;
  try {
    await vault.linkTaskToDay(dayMs: dayMs, taskPageId: taskPageId);
  } catch (error) {
    debugPrint('Task Journal link failed for $taskPageId on $dayMs: $error');
  }
}
