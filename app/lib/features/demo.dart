// The direction brief's illustrative vault, with dates relative to today.
// Seeded only in a fresh vault with --dart-define=SEED_DEMO=true.

import 'dart:convert';

import '../core/clock.dart';
import '../core/models.dart';
import '../core/vault.dart';
import '../editor/mention_menu.dart' show isoDate, longDate;
import '../ui/database/values.dart';
import 'getting_started.dart';

const demoSpaces = ['Home life', 'Work', 'Trips', 'Reading', 'Garden'];
const demoTaskTitles = [
  'Book Lisbon flights',
  'Feed sourdough starter',
  'Draft Q4 goals',
  'Plant garlic',
  'Return library books',
  'Call plumber',
  'Choose Lisbon neighbourhood',
  'Review roadmap with team',
  'Pick up compost',
  'Read the next chapter',
  'Plan meals for the week',
  'Send trip dates',
];
const packingItems = [
  'Passport',
  'Flights confirmed',
  'Walking shoes',
  'Light jacket',
  'Phone charger',
  'Toiletries',
  'Book for the flight',
];

class _Blocks {
  final ops = <BOp>[];
  final _last = <String?, String>{};

  String add(String kind, String text, {String? parent, bool checked = false, List<BMark> marks = const [], String? target}) {
    final id = newId();
    ops.add(BOp(op: OpKind.insert, id: id, parent: parent, after: _last[parent], kind: kind, text: text, marks: marks, checked: checked));
    if (target != null) {
      ops.add(BOp(op: OpKind.setTarget, id: id, kind: kind, text: '', marks: const [], checked: false, target: target));
    }
    _last[parent] = id;
    return id;
  }

  void dateTodo(String title, DateTime date, {bool checked = false}) {
    final mention = '@${longDate(date)}';
    final text = '$title $mention';
    add(
      Kinds.todo,
      text,
      checked: checked,
      marks: [BMark(start: title.length + 1, end: text.length, kind: BMarkKind.date, url: isoDate(date))],
    );
  }

  void mention(String prefix, String title, String target) {
    final text = '$prefix$title';
    add(
      Kinds.paragraph,
      text,
      marks: [BMark(start: prefix.length, end: text.length, kind: BMarkKind.pageLink, url: target)],
    );
  }
}

PropDef _def(String name, PropKind kind, {List<SelectOption> options = const [], String config = ''}) =>
    PropDef(key: '', name: name, kind: kind, options: options, config: config);

Future<String> _collection(Vault vault, String parent, String title, List<PropDef> schema) async {
  final id = await vault.createPage(parent: parent, title: title);
  await vault.run((v) => v.makeCollection(page: id), writes: true);
  await vault.run((v) => v.setSchema(collection: id, schema: schema), writes: true);
  return id;
}

/// Returns title -> page id for deterministic tests and fixture navigation.
/// Boot ignores the map; the seed consists entirely of normal pages/blocks.
Future<Map<String, String>> createDemo(Vault vault) async {
  final now = clockNow();
  final today = Day.of(now);
  DateTime day(int delta) {
    final d = today.add(delta);
    return DateTime(d.year, d.month, d.day);
  }

  final ids = <String, String>{};
  Future<String> page(String title, {String? parent, String? icon}) async {
    final id = await vault.createPage(title: title, parent: parent);
    ids[title] = id;
    if (icon != null) await vault.setIcon(id, icon);
    return id;
  }

  const colors = ['apricot', 'duvet', 'paw', 'sky', 'moss'];
  const icons = ['🏡', '💼', '🧳', '📚', '🌱'];
  for (var i = 0; i < demoSpaces.length; i++) {
    final id = await page(demoSpaces[i], icon: icons[i]);
    await vault.run((v) => v.setPageColor(page: id, color: colors[i]), writes: true);
  }

  final tasks = await createTasksDatabase(vault, parent: ids['Work'], sampleRows: false);
  ids['Tasks'] = tasks;
  final taskSchema = await vault.run((v) => v.getSchema(collection: tasks));
  final status = taskSchema.firstWhere((d) => d.name == 'Status');
  final due = taskSchema.firstWhere((d) => d.name == 'Due');
  const offsets = [-3, 0, 0, 0, -1, -2, 1, 2, 3, 4, 0, 1];
  const statuses = [0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 2];
  for (var i = 0; i < demoTaskTitles.length; i++) {
    final row = await page(demoTaskTitles[i], parent: tasks);
    await vault.run((v) => v.setProperty(page: row, key: status.key, value: Values.status(status.options[statuses[i]].id)), writes: true);
    await vault.run((v) => v.setProperty(page: row, key: due.key, value: Values.date(today.add(offsets[i]).ms)), writes: true);
  }

  final calendar = await createCalendarDatabase(vault, parent: ids['Work'], sampleRows: false);
  ids['Calendar'] = calendar;
  final when = (await vault.run((v) => v.getSchema(collection: calendar))).single.key;
  for (final (title, hour, minute, minutes) in const [('Standup', 9, 30, 30), ('Dentist', 14, 0, 45), ('Pottery class', 19, 0, 90)]) {
    final row = await page(title, parent: calendar);
    final start = DateTime(now.year, now.month, now.day, hour, minute);
    await vault.run(
      (v) => v.setProperty(
        page: row,
        key: when,
        value: Values.date(start.millisecondsSinceEpoch, end: start.add(Duration(minutes: minutes)).millisecondsSinceEpoch, hasTime: true),
      ),
      writes: true,
    );
  }

  final roadmap = await _collection(vault, ids['Work']!, 'Q4 roadmap', [
    _def(
      'Status',
      PropKind.status,
      options: const [
        SelectOption(id: '', name: 'To do', color: 'gray'),
        SelectOption(id: '', name: 'Doing', color: 'blue'),
        SelectOption(id: '', name: 'Done', color: 'green'),
      ],
    ),
    _def(
      'Priority',
      PropKind.select,
      options: const [
        SelectOption(id: '', name: 'High', color: 'red'),
        SelectOption(id: '', name: 'Medium', color: 'yellow'),
      ],
    ),
  ]);
  ids['Q4 roadmap'] = roadmap;
  await vault.setIcon(roadmap, '🗺️');
  for (final title in ['Capture from anywhere', 'A calmer daily view', 'Reliable offline editing']) {
    await page(title, parent: roadmap);
  }

  final recipes = await _collection(vault, ids['Home life']!, 'Recipes', [_def('Notes', PropKind.text)]);
  ids['Recipes'] = recipes;
  await vault.setIcon(recipes, '🍲');
  final sourdough = await page('Sourdough', parent: recipes, icon: '🍞');
  final recipeBlocks = _Blocks()
    ..add(Kinds.heading2, 'An everyday loaf')
    ..add(Kinds.paragraph, '100 g starter, 350 g water, 500 g bread flour, 10 g salt.')
    ..add(Kinds.numbered, 'Mix, then rest for 30 minutes.')
    ..add(Kinds.numbered, 'Fold four times, let rise, then shape.')
    ..add(Kinds.numbered, 'Chill overnight and bake in a hot covered pot.');
  await vault.apply(sourdough, recipeBlocks.ops);

  final books = await _collection(vault, ids['Reading']!, 'Book list', [_def('Author', PropKind.text)]);
  ids['Book list'] = books;
  await vault.setIcon(books, '📖');
  for (final title in ['The Book of Delights', 'Braiding Sweetgrass', 'A Field Guide to Getting Lost']) {
    await page(title, parent: books);
  }

  final weekly = await page('Weekly review', parent: ids['Work'], icon: '📝');
  final garden = await page('Garden plan', parent: ids['Garden'], icon: '🌿');
  final gardenBlocks = _Blocks()
    ..add(Kinds.heading2, 'October in the garden')
    ..add(Kinds.paragraph, 'Make space for the next season: garlic, compost and a few hardy herbs.')
    ..dateTodo('Plant garlic', day(0))
    ..dateTodo('Turn the compost', day(2))
    ..dateTodo('Water the seedlings', day(0), checked: true);
  await vault.apply(garden, gardenBlocks.ops);

  final lisbon = await page('Lisbon in May', parent: ids['Trips'], icon: '🚋');
  // A small, local solid-colour cover. No network or generated asset paths.
  final cover = await vault.run(
    (v) => v.addAttachmentBytes(
      name: 'lisbon-cover.png',
      bytes: base64Decode(
        'iVBORw0KGgoAAAANSUhEUgAAAGAAAAAwCAIAAABhdOiYAAAAZElEQVR4nO3QMQ0AIADAMJxiCn1owQGc7GgyAUvHXlOXxveDeIAAAQIEKBwgQIAAAQoHCBAgQIDCAQIECBCgcIAAAQIEKBwgQIAAAQoHCBAgQIDCAQIECBCgcIAAAQIEKBygRwe/TbPhnDHi2gAAAABJRU5ErkJggg==',
      ),
    ),
    writes: true,
  );
  await vault.run((v) => v.setCover(id: lisbon, cover: cover), writes: true);

  final bookings = await _collection(vault, lisbon, 'Bookings', [
    _def(
      'Status',
      PropKind.status,
      options: const [
        SelectOption(id: '', name: 'To book', color: 'gray'),
        SelectOption(id: '', name: 'Booked', color: 'green'),
      ],
    ),
  ]);
  ids['Bookings'] = bookings;
  final bookingStatus = (await vault.run((v) => v.getSchema(collection: bookings))).single;
  final bookingViews = await vault.run((v) => v.listViews(collection: bookings));
  await vault.run(
    (v) => v.updateView(
      collection: bookings,
      view: bookingViews.first.copyWith(name: 'Board', kind: ViewKind.board, groupBy: bookingStatus.key),
    ),
    writes: true,
  );
  for (final (title, done) in const [('Apartment in Alfama', true), ('Train to Sintra', false), ('Dinner at Prado', false)]) {
    final row = await page(title, parent: bookings);
    await vault.run(
      (v) => v.setProperty(page: row, key: bookingStatus.key, value: Values.status(bookingStatus.options[done ? 1 : 0].id)),
      writes: true,
    );
  }

  final packing = await page('Packing checklist', parent: ids['Trips'], icon: '🎒');
  final packingBlocks = _Blocks()..add(Kinds.heading2, 'Packing');
  for (var i = 0; i < packingItems.length; i++) {
    packingBlocks.add(Kinds.todo, packingItems[i], checked: i < 2);
  }
  await vault.apply(packing, packingBlocks.ops);

  final rich = _Blocks();
  rich.add(Kinds.callout, 'Flights confirmed. Keep the confirmation and travel documents here.', target: '✈️');
  final columns = rich.add(Kinds.columnList, '');
  final see = rich.add(Kinds.column, '', parent: columns);
  final eat = rich.add(Kinds.column, '', parent: columns);
  rich.add(Kinds.heading2, 'Must see', parent: see);
  for (final title in ['Alfama at sunrise', 'Gulbenkian Museum', 'A day in Sintra']) {
    rich.add(Kinds.bulleted, title, parent: see);
  }
  rich.add(Kinds.heading2, 'Must eat', parent: eat);
  for (final title in ['Pastéis de nata', 'Grilled sardines', 'Dinner at Prado']) {
    rich.add(Kinds.bulleted, title, parent: eat);
  }
  rich.add(Kinds.heading2, 'Packing');
  for (var i = 0; i < packingItems.length; i++) {
    rich.add(Kinds.todo, packingItems[i], checked: i < 2);
  }
  rich.add(Kinds.linkToPage, '', target: packing);
  final daily = rich.add(Kinds.toggle, 'Day by day');
  rich.add(
    Kinds.paragraph,
    '12 May: arrive and settle in. 13 May: explore Alfama. 14 May: museums and a slow lunch. Keep the rest open.',
    parent: daily,
  );
  rich.add('database', '', target: bookings);
  rich.mention('Discuss the plan in ', 'Weekly review', weekly);
  rich.add(Kinds.image, 'A favourite Lisbon view goes here.');
  await vault.apply(lisbon, rich.ops);

  final reviewBlocks = _Blocks()
    ..add(Kinds.heading2, 'This week')
    ..add(Kinds.paragraph, 'A little work, some time outdoors, and something to look forward to.')
    ..mention('Make room for ', 'Lisbon in May', lisbon)
    ..mention('Check on the ', 'Garden plan', garden);
  await vault.apply(weekly, reviewBlocks.ops);
  for (final id in [lisbon, garden, weekly]) {
    await vault.setFavorite(id, true);
  }
  return ids;
}
