// The `/` menu entries for schedules. Like the kanban entry, they only insert:
// the editor stores the kit's embed kind and calls createTarget, which makes
// the schedule note under the page and points the embed at it.

import 'package:flutter/widgets.dart';

import '../../registry/block_codecs.dart';
import '../../registry/block_registry.dart';
import '../../registry/embed_block.dart';
import '../../ui/icon_x.dart';

/// Block kind of a plain schedule (no step, no runner yet).
const scheduleInsertKind = 'schedule';

/// Block kind of the weekly note schedule.
const scheduleWeeklyInsertKind = 'schedule_weekly';

BlockType scheduleInsertBlock({
  required String kind,
  required String label,
  required String hint,
  required String aliases,
  required Future<String?> Function(BuildContext context, String pageId)
  createTarget,
}) => BlockType(
  kind: kind,
  label: label,
  hint: hint,
  iconX: Ix.calendar,
  aliases: aliases,
  group: 'Advanced',
  editable: false,
  acceptsChildren: false,
  colorable: false,
  insertOnly: true,
  insertKind: embedKind,
  createTarget: createTarget,
  body: (_) => const SizedBox.shrink(),
  codecs: genericBlockCodecs,
);
