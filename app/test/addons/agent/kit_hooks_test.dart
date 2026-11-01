// The kit pane's Scratch hooks over synthetic activity groups.
import 'package:agent_kit_core/agent_kit_core.dart' as kit;
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_transcript.dart';
import 'package:notes_app/addons/agent/agent_turn.dart';
import 'package:notes_app/addons/agent/kit_hooks.dart';

kit.ToolRow _row(
  String id,
  String name, {
  kit.WorkState state = kit.WorkState.done,
  String? arguments,
  String? output,
}) => kit.ToolRow(
  id: id,
  name: name,
  state: state,
  arguments: arguments,
  output: output == null
      ? null
      : kit.ToolOutput(text: output, totalBytes: output.length),
);

void main() {
  test('the verb line counts tools by name and failed calls', () {
    final g = kit.ActivityGroup(
      id: 'a',
      rows: [
        _row('1', 'search_notes'),
        _row('2', 'search_notes'),
        _row('3', 'read_page', state: kit.WorkState.failed),
      ],
    );
    expect(kitVerbSummary(g), 'Searched 2× · Read 1 page · 1 failed');
    expect(verbSummary(['edit_page'], 0), 'Edited 1 page');
  });

  test('Show code waits for the group to finish', () {
    final running = kit.ActivityGroup(
      id: 'a',
      rows: [_row('1', 'js', state: kit.WorkState.running)],
    );
    expect(kitCodeFooter(running), isNull);
    expect(kitCodeFooter(kit.ActivityGroup(id: 'b', rows: [])), isNull);
    final done = kit.ActivityGroup(
      id: 'c',
      rows: [_row('1', 'js', arguments: '{"code":"1+1"}', output: '2')],
    );
    expect(kitCodeFooter(done), isA<CodeDisclosure>());
  });

  test('a code cell reads its code from the arguments', () {
    expect(codeOfArguments('{"code":"x"}'), 'x');
    expect(codeOfArguments('{"a":1}'), '{"a":1}');
  });
}
