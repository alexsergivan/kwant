//! The recipe block: the contract example, recipe-shaped replies, near misses, the qty/text split
//! and the checks. All data is made up.
use serde_json::json;

use super::*;
use crate::server::smart_results::{compute, validate, Block, ResultSet, Source, VERSION};

fn first(text: &str) -> Option<Block> {
    compute(text).map(|set| set.blocks.into_iter().next().expect("one block"))
}

fn kind(text: &str) -> Option<&'static str> {
    first(text).map(|b| b.kind())
}

fn recipe_in(text: &str) -> Recipe {
    match first(text) {
        Some(Block::Recipe(r)) => r,
        other => panic!("no recipe: {other:?}"),
    }
}

/// (qty, text) of every ingredient.
fn split(r: &Recipe) -> Vec<(Option<&str>, &str)> {
    r.ingredients.iter().map(|i| (i.qty.as_deref(), i.text.as_str())).collect()
}

fn src(r: &Recipe) -> (usize, bool, usize, bool, usize) {
    let s = &r.source;
    assert_eq!(s.kind, "markdown_section");
    (s.index.unwrap(), s.ordered.unwrap(), s.last_index.unwrap(), s.last_ordered.unwrap(), s.lead.unwrap())
}

const CONTRACT: &str = "Here's a cosy stew for tonight.

## Golden squash and lentil stew

**Serves 4** · Prep 15 min · Cook 35 min

### Ingredients

**For the stew**
- 1 tbsp olive oil
- 1 onion, diced
- 2 cloves garlic, crushed
- 500 g squash, peeled and cubed
- 1 can (400 g) chopped tomatoes
- Salt to taste

**For the topping:**
- ½ cup plain yogurt
- 1–2 tsp chili flakes

### Method

1. Warm the oil in a large pot over medium heat. Add the onion and cook for 5 minutes, until soft.
2. Stir in the garlic and cook for 1 minute.
3. Add the squash and tomatoes and simmer for 25 minutes.
4. Serve with the yogurt and chili flakes.

### Notes

- Keeps 3 days in the fridge.
- Sweet potato works in place of the squash.

Want a vegan version?
";

// --- the contract

#[test]
fn the_contract_example_is_exact() {
    let set = compute(CONTRACT).expect("a block");
    let expected = json!({
      "version": 1,
      "blocks": [{
        "type": "recipe",
        "id": "b0",
        "title": "Golden squash and lentil stew",
        "source": { "kind": "markdown_section", "index": 0, "ordered": false, "last_index": 2, "last_ordered": false, "lead": 4 },
        "serves": 4,
        "times": [ { "label": "Prep", "text": "15 min" }, { "label": "Cook", "text": "35 min" } ],
        "ingredients": [
          { "id": "i0", "text": "olive oil", "qty": "1 tbsp", "group": "For the stew" },
          { "id": "i1", "text": ", diced", "qty": "1 onion", "group": "For the stew" },
          { "id": "i2", "text": "garlic, crushed", "qty": "2 cloves", "group": "For the stew" },
          { "id": "i3", "text": "squash, peeled and cubed", "qty": "500 g", "group": "For the stew" },
          { "id": "i4", "text": "(400 g) chopped tomatoes", "qty": "1 can", "group": "For the stew" },
          { "id": "i5", "text": "Salt to taste", "qty": null, "group": "For the stew" },
          { "id": "i6", "text": "plain yogurt", "qty": "½ cup", "group": "For the topping" },
          { "id": "i7", "text": "chili flakes", "qty": "1–2 tsp", "group": "For the topping" }
        ],
        "steps": [
          { "id": "s0", "text": "Warm the oil in a large pot over medium heat. Add the onion and cook for 5 minutes, until soft." },
          { "id": "s1", "text": "Stir in the garlic and cook for 1 minute." },
          { "id": "s2", "text": "Add the squash and tomatoes and simmer for 25 minutes." },
          { "id": "s3", "text": "Serve with the yogurt and chili flakes." }
        ],
        "notes": [ "Keeps 3 days in the fridge.", "Sweet potato works in place of the squash." ],
        "provenance": { "kind": "reply_text" }
      }]
    });
    assert_eq!(serde_json::to_value(&set).unwrap(), expected);
    // And it reads back as the same block
    let back: ResultSet = serde_json::from_value(expected).unwrap();
    assert_eq!(back, set);
    assert!(validate(&back, Some(CONTRACT)).is_ok());
}

#[test]
fn every_ingredient_line_is_qty_and_text_joined() {
    let r = recipe_in(CONTRACT);
    let lines: Vec<String> = r.ingredients.iter().map(Ingredient::line).collect();
    assert_eq!(lines[1], "1 onion, diced");
    assert_eq!(lines[4], "1 can (400 g) chopped tomatoes");
    for l in &lines {
        assert!(CONTRACT.contains(&format!("- {l}\n")), "{l}");
    }
}

// --- recipe-shaped replies

#[test]
fn a_plain_recipe_with_a_heading_and_instructions() {
    let r = recipe_in(
        "## Fluffy pancakes\n\n### Ingredients\n\n- 1 cup flour\n- 2 eggs\n- 1 cup milk\n- 1 tsp baking powder\n\n### Instructions\n\n1. Whisk everything together.\n2. Cook ladlefuls in a hot pan, 2 minutes a side.\n",
    );
    assert_eq!(r.title.as_deref(), Some("Fluffy pancakes"));
    assert_eq!((r.serves, r.times.len(), r.notes.len()), (None, 0, 0));
    assert_eq!(src(&r), (0, false, 0, true, 2));
    assert_eq!(split(&r)[1], (Some("2"), "eggs"));
    assert_eq!(r.steps[1].text, "Cook ladlefuls in a hot pan, 2 minutes a side.");
    assert!(r.ingredients.iter().all(|i| i.group.is_none()));
}

#[test]
fn groups_under_headings_and_bold_labels() {
    let text = "Here you go!\n\n# Lemon tart\n\n## Ingredients\n\n#### For the pastry\n- 200g plain flour\n- 100 g cold butter, cubed\n- 1 egg yolk\n\n#### For the filling\n- 3 lemons, zest and juice\n- 150 g sugar\n- 3 eggs\n\n__Topping:__\n- Icing sugar, to dust\n\n## Method\n1. Rub the butter into the flour, add the yolk and chill for 30 minutes.\n2. Roll out, line a tin and blind bake for 15 minutes.\n3. Whisk the filling, pour in and bake for 25 minutes.\n";
    let r = recipe_in(text);
    assert_eq!(r.title.as_deref(), Some("Lemon tart"));
    let groups: Vec<_> = r.ingredients.iter().map(|i| i.group.as_deref()).collect();
    assert_eq!(groups, [Some("For the pastry"), Some("For the pastry"), Some("For the pastry"), Some("For the filling"), Some("For the filling"), Some("For the filling"), Some("Topping")]);
    assert_eq!(split(&r)[0], (Some("200g"), "plain flour"));
    assert_eq!(split(&r)[3], (Some("3 lemons"), ", zest and juice"));
    assert_eq!(split(&r)[6], (None, "Icing sugar, to dust"));
    // # Lemon tart, ## Ingredients, #### For the pastry, then ul#0; last is ol#0
    assert_eq!(src(&r), (0, false, 0, true, 3));
}

#[test]
fn ranges_and_fractions_keep_the_amount_as_written() {
    let r = recipe_in(
        "### Ingredients\n- 1–2 tsp chili flakes\n- 1/2 cup rice\n- 1 1/2 cups stock\n- 1½ cups water\n- 2-3 cloves garlic\n- 1 - 2 tbsp. soy sauce\n- ¼ tsp salt\n- 1,5 l broth\n\n### Steps\n1. Toast the rice.\n2. Add everything else and simmer.\n",
    );
    assert_eq!(
        split(&r),
        [
            (Some("1–2 tsp"), "chili flakes"),
            (Some("1/2 cup"), "rice"),
            (Some("1 1/2 cups"), "stock"),
            (Some("1½ cups"), "water"),
            (Some("2-3 cloves"), "garlic"),
            (Some("1 - 2 tbsp."), "soy sauce"),
            (Some("¼ tsp"), "salt"),
            (Some("1,5 l"), "broth"),
        ]
    );
    assert_eq!(r.title, None);
    assert_eq!(src(&r).4, 1);
}

#[test]
fn no_serves_no_times_no_title() {
    let r = recipe_in("**Ingredients:**\n- 2 slices bread\n- 1 tbsp butter\n\n**Method:**\n1. Butter the bread.\n2. Toast it.\n\nEnjoy!\n");
    assert_eq!((r.title.as_deref(), r.serves, r.times.len()), (None, None, 0));
    assert_eq!(src(&r), (0, false, 0, true, 1));
}

#[test]
fn servings_in_the_cue_and_a_numbered_method_straight_after() {
    let r = recipe_in("A quick one.\n\nIngredients (serves 2):\n- 2 eggs\n- 1 knob butter\n- Salt and pepper\n\n1. Beat the eggs.\n2. Melt the butter and stir the eggs until just set.\n");
    assert_eq!(r.serves, Some(2));
    assert_eq!(split(&r)[1], (Some("1 knob"), "butter"));
    assert_eq!(src(&r), (0, false, 0, true, 1));
}

#[test]
fn an_intro_line_naming_the_ingredients_stays_text() {
    let text = "## Hummus\n\nHere are the ingredients you'll need:\n\n- 1 can chickpeas, drained\n- 2 tbsp tahini\n- 1 lemon, juiced\n\nThen follow these steps:\n\n1. Blend everything until smooth.\n2. Loosen with water.\n";
    let r = recipe_in(text);
    // The section starts at the list: the heading and the intro stay above the card
    assert_eq!(r.title, None);
    assert_eq!(src(&r), (0, false, 0, true, 0));
    assert_eq!(split(&r)[0], (Some("1 can"), "chickpeas, drained"));
}

#[test]
fn a_meta_list_and_times_above_the_cue() {
    let text = "### Green salad\n\n- **Serves:** 2\n- **Prep time:** 10 minutes\n- Total: 1 hr 10 min\n\n### Ingredients\n- 1 head lettuce\n- 2 tbsp olive oil\n\n### Directions\n1. Wash and dry the lettuce.\n2. Dress and serve.\n";
    let r = recipe_in(text);
    assert_eq!(r.title.as_deref(), Some("Green salad"));
    assert_eq!(r.serves, Some(2));
    let times: Vec<_> = r.times.iter().map(|t| (t.label.as_str(), t.text.as_str())).collect();
    assert_eq!(times, [("Prep time", "10 minutes"), ("Total", "1 hr 10 min")]);
    // The meta list is the section's first list (ul#0), the method ol#0; lead 1 (the title heading)
    assert_eq!(src(&r), (0, false, 0, true, 1));
}

#[test]
fn tips_after_the_method_and_task_boxes_on_ingredients() {
    let text = "**Overnight oats**\n\nServes 1 | Chill: overnight\n\nIngredients\n- [ ] 1/2 cup oats\n- [x] 1/2 cup milk\n- [ ] 1 tbsp honey\n\nInstructions\n1. Stir everything in a jar.\n2. Chill overnight.\n\n**Tips:**\n- Add berries in the morning.\n";
    let r = recipe_in(text);
    assert_eq!(r.title.as_deref(), Some("Overnight oats"));
    assert_eq!((r.serves, r.times[0].label.as_str(), r.times[0].text.as_str()), (Some(1), "Chill", "overnight"));
    assert_eq!(split(&r)[0], (Some("1/2 cup"), "oats"));
    assert_eq!(r.notes, ["Add berries in the morning."]);
    assert_eq!(src(&r), (0, false, 1, false, 3));
}

#[test]
fn task_boxes_on_notes_are_dropped_like_on_ingredients() {
    // F-4: a task-list Tips list loses its boxes, as the ingredients do
    let text = "### Garlic bread

### Ingredients
- [ ] 1 baguette
- [x] 3 cloves garlic

### Method
1. Spread the garlic butter.
2. Bake.

### Tips
- [ ] Use fresh garlic
- [x] Wrap in foil to keep it soft
- [X] Serve warm
- Plain tip
";
    let r = recipe_in(text);
    assert_eq!(split(&r)[1], (Some("3 cloves"), "garlic"));
    assert_eq!(r.notes, ["Use fresh garlic", "Wrap in foil to keep it soft", "Serve warm", "Plain tip"]);
    assert!(r.ingredients.iter().all(|i| !i.text.contains('[')));
}

#[test]
fn a_setext_title_and_a_bullet_method_under_its_heading() {
    let text = "Tomato soup\n===========\n\nIngredients\n-----------\n\n- 1 kg tomatoes\n- 1 onion\n- 500 ml stock\n\nMethod\n------\n\n- Soften the onion.\n- Add the tomatoes and stock, simmer 20 minutes, blend.\n";
    let r = recipe_in(text);
    assert_eq!(r.title.as_deref(), Some("Tomato soup"));
    assert_eq!(split(&r)[1], (Some("1"), "onion"));
    assert_eq!(src(&r), (0, false, 1, false, 2));
}

#[test]
fn a_meta_line_it_cannot_read_stays_text_above_the_card() {
    // "Serves 4–6" is no single count: the line and the title above it stay text
    let text = "## Chili\n\nServes 4–6\n\n### Ingredients\n- 500 g beans\n- 2 tsp cumin\n\n### Method\n1. Fry the spices.\n2. Add the beans and simmer.\n";
    let r = recipe_in(text);
    assert_eq!((r.title.as_deref(), r.serves), (None, None));
    assert_eq!(src(&r).4, 1);
}

#[test]
fn a_recipe_ranks_above_a_table_and_the_steps_it_holds() {
    let text = "| Pan | Size |\n|---|---|\n| Small | 20 cm |\n| Large | 28 cm |\n| Tray | 30 cm |\n\n### Ingredients\n- 2 cups flour\n- 1 cup sugar\n\n### Method\n1. Mix.\n2. Bake for 30 minutes.\n3. Cool.\n";
    assert_eq!(kind(text), Some("recipe"));
}

// --- near misses

#[test]
fn a_shopping_list_alone_is_not_a_recipe() {
    assert_ne!(kind("Here's your shopping list:\n\n- 2 onions\n- 1 kg potatoes\n- Milk\n- 6 eggs\n"), Some("recipe"));
    assert_ne!(kind("### Ingredients\n- 2 onions\n- 1 kg potatoes\n- Milk\n\nBuy them at the market.\n"), Some("recipe"));
}

#[test]
fn a_packing_list_is_not_a_recipe() {
    let text = "### Packing list\n- Tent\n- 2 sleeping bags\n- 1 stove\n\n### Before you go\n1. Check the weather.\n2. Tell someone your route.\n3. Charge your phone.\n";
    assert_ne!(kind(text), Some("recipe"));
}

#[test]
fn a_how_to_without_ingredients_stays_steps() {
    let text = "### What you need\n- A drill\n- 4 screws\n\n### Steps\n1. Mark the holes.\n2. Drill them.\n3. Screw the shelf on.\n";
    assert_eq!(kind(text), Some("steps"));
    assert_eq!(kind("How to reset it:\n\n1. Unplug the router.\n2. Wait 30 seconds.\n3. Plug it back in.\n"), Some("steps"));
}

#[test]
fn a_meal_plan_with_several_dishes_is_not_a_recipe() {
    let two = "## Monday: pasta\n\n**Ingredients:**\n- 200 g pasta\n- 1 jar pesto\n\n**Method:**\n1. Boil the pasta.\n2. Stir in the pesto.\n\n## Tuesday: soup\n\n**Ingredients:**\n- 1 kg carrots\n- 1 l stock\n\n1. Simmer.\n2. Blend.\n";
    assert_ne!(kind(two), Some("recipe"));
    let by_day = "### Ingredients\n- 1 kg pasta\n- 2 jars sauce\n- 1 bag salad\n\n1. Monday: pasta with sauce\n2. Tuesday: salad\n3. Wednesday: leftovers\n";
    assert_ne!(kind(by_day), Some("recipe"));
}

#[test]
fn a_second_dish_without_its_own_cue_is_not_one_recipe() {
    let text = "## Rice\n\n### Ingredients\n- 1 cup rice\n- 2 cups water\n\n### Method\n1. Rinse.\n2. Simmer 15 minutes.\n\n## And a dal\n\n- 1 cup lentils\n- 3 cups water\n\n1. Rinse.\n2. Simmer 25 minutes.\n";
    assert_ne!(kind(text), Some("recipe"));
}

#[test]
fn a_weather_reply_is_not_a_recipe() {
    let text = "**Today:** 18°C, light rain\n**Tomorrow:** 21°C, sunny\n\n- Bring an umbrella in the morning.\n- Wind 12 km/h.\n";
    assert_ne!(kind(text), Some("recipe"));
}

#[test]
fn a_method_in_prose_or_a_block_in_between_is_not_a_card() {
    let prose = "### Ingredients\n- 2 eggs\n- 1 cup milk\n\n### Method\nWhisk them and cook in a hot pan for 2 minutes.\n";
    assert_ne!(kind(prose), Some("recipe"));
    let code = "### Ingredients\n- 2 eggs\n- 1 cup milk\n\n```\noven: 180C\n```\n\n1. Whisk.\n2. Bake.\n";
    assert_ne!(kind(code), Some("recipe"));
    let para = "### Ingredients\n- 2 eggs\n- 1 cup milk\n\nYou can swap the milk for water if you like, it works fine.\n\n1. Whisk.\n2. Bake.\n";
    assert_ne!(kind(para), Some("recipe"));
}

#[test]
fn a_cue_it_cannot_place_or_a_split_method_is_not_a_card() {
    let mixed = "## Ingredients and method\n- 2 eggs\n- 1 cup milk\n\n1. Whisk.\n2. Bake.\n";
    assert_ne!(kind(mixed), Some("recipe"));
    let split_method = "### Ingredients\n- 2 eggs\n- 1 cup milk\n\n### Method\n1. Whisk.\n2. Bake.\n\n**For the glaze:**\n1. Melt.\n2. Pour.\n";
    assert_ne!(kind(split_method), Some("recipe"));
    let one = "### Ingredients\n- 2 eggs\n\n### Method\n1. Boil.\n2. Peel.\n";
    assert_ne!(kind(one), Some("recipe"));
}

#[test]
fn html_in_a_step_is_not_a_card() {
    let text = "### Ingredients\n- 2 eggs\n- 1 cup milk\n\n### Method\n1. Whisk <b>well</b>.\n2. Bake.\n";
    assert_ne!(kind(text), Some("recipe"));
}

// --- qty and text

#[test]
fn split_qty_takes_the_leading_amount_as_written() {
    let cases: &[(&str, Option<&str>, &str)] = &[
        ("2 cups flour, sifted", Some("2 cups"), "flour, sifted"),
        ("1 onion, diced", Some("1 onion"), ", diced"),
        ("3 eggs", Some("3"), "eggs"),
        ("2 cups", Some("2"), "cups"),
        ("2 large onions", Some("2"), "large onions"),
        ("200g flour", Some("200g"), "flour"),
        ("250ml milk", Some("250ml"), "milk"),
        ("1 tbsp. olive oil", Some("1 tbsp."), "olive oil"),
        ("1 Tbsp olive oil", Some("1 Tbsp"), "olive oil"),
        ("1,000 g water", Some("1,000 g"), "water"),
        ("½ cup yogurt", Some("½ cup"), "yogurt"),
        ("1 ½ cups milk", Some("1 ½ cups"), "milk"),
        ("2 1/2 cups flour", Some("2 1/2 cups"), "flour"),
        ("12–15 cherry tomatoes", Some("12–15"), "cherry tomatoes"),
        ("2 x 400g tins tomatoes", Some("2"), "x 400g tins tomatoes"),
        ("1 can (400 g) tomatoes", Some("1 can"), "(400 g) tomatoes"),
        ("Salt to taste", None, "Salt to taste"),
        ("A pinch of salt", None, "A pinch of salt"),
        ("Zest of 1 lemon", None, "Zest of 1 lemon"),
        ("**2 cups** flour", None, "**2 cups** flour"),
        ("1-inch piece ginger", None, "1-inch piece ginger"),
        ("2x eggs", None, "2x eggs"),
        ("2  cups flour", None, "2  cups flour"),
        ("3", None, "3"),
        ("500g/1lb beef", None, "500g/1lb beef"),
        // A temperature is not an amount (F-2): no qty, so it never scales
        ("350 °F oven", None, "350 °F oven"),
        ("350°F oven", None, "350°F oven"),
        ("180 °C fan oven", None, "180 °C fan oven"),
        ("180ºC oven", None, "180ºC oven"),
        ("200 ℃ oven", None, "200 ℃ oven"),
        ("350F oven", None, "350F oven"),
        ("350 F oven", None, "350 F oven"),
        ("180 C fan", None, "180 C fan"),
        ("100 C", None, "100 C"),
        ("90 C oven", None, "90 C oven"),
        ("350 degrees oven", None, "350 degrees oven"),
        ("180 Degrees C", None, "180 Degrees C"),
        ("350–375 °F oven", None, "350–375 °F oven"),
        ("400 deg. oven", None, "400 deg. oven"),
        ("180 Celsius oven", None, "180 Celsius oven"),
        // Near misses that stay amounts: a unit or word that only starts with F or C
        ("2 cups flour", Some("2 cups"), "flour"),
        ("2 Cups flour", Some("2 Cups"), "flour"),
        ("1 can chickpeas", Some("1 can"), "chickpeas"),
        ("2 c. flour", Some("2"), "c. flour"),
        ("3 Fuji apples", Some("3"), "Fuji apples"),
        ("1 cl rum", Some("1 cl"), "rum"),
        ("4 fl oz cream", Some("4"), "fl oz cream"),
        // A lone capital C under 90 is a US cup, so it still scales
        ("1 C sugar", Some("1"), "C sugar"),
        ("2½ C flour", Some("2½"), "C flour"),
        ("1 1/2 C milk", Some("1 1/2"), "C milk"),
        ("½ C butter", Some("½"), "C butter"),
        ("89 C water", Some("89"), "C water"),
    ];
    for (line, qty, text) in cases {
        let got = split_qty(line);
        assert_eq!(got, (*qty, *text), "{line}");
        let i = Ingredient { id: "i0".into(), text: got.1.into(), qty: got.0.map(Into::into), group: None };
        assert_eq!(i.line(), *line, "{line} rebuilds");
    }
}

// --- the checks

fn contract_set() -> ResultSet {
    compute(CONTRACT).expect("a block")
}

fn tampered(f: impl FnOnce(&mut Recipe)) -> ResultSet {
    let mut set = contract_set();
    let Block::Recipe(r) = &mut set.blocks[0] else { panic!() };
    f(r);
    set
}

#[test]
fn validate_holds_a_recipe_to_its_limits() {
    let bad: Vec<(&str, ResultSet)> = vec![
        ("serves 0", tampered(|r| r.serves = Some(0))),
        ("serves 25", tampered(|r| r.serves = Some(25))),
        ("qty", tampered(|r| r.ingredients[0].qty = Some("a lot".into()))),
        ("ingredient id", tampered(|r| r.ingredients[1].id = "i7".into())),
        ("step id", tampered(|r| r.steps[0].id = "x".into())),
        ("one step", tampered(|r| r.steps.truncate(1))),
        ("one ingredient", tampered(|r| r.ingredients.truncate(1))),
        ("7 notes", tampered(|r| r.notes = vec!["Keeps 3 days in the fridge.".into(); 7])),
        ("4 times", tampered(|r| r.times = vec![r.times[0].clone(); 4])),
        ("empty step", tampered(|r| r.steps[0].text = " ".into())),
        ("html", tampered(|r| r.steps[0].text = "Warm <script>x</script>".into())),
        ("list source", tampered(|r| r.source = Source::list(0, false))),
        ("lead", tampered(|r| r.source.lead = Some(13))),
        ("ends before it starts", tampered(|r| r.source.index = Some(3))),
        ("no last list", tampered(|r| r.source.last_index = None)),
        ("long step", tampered(|r| r.steps[0].text = "a".repeat(601))),
        ("provenance", tampered(|r| r.provenance = Provenance::kwant_data("x".into()))),
    ];
    for (what, set) in bad {
        assert!(validate(&set, None).is_err(), "{what}");
    }
    assert!(validate(&contract_set(), None).is_ok());
}

#[test]
fn every_recipe_text_must_be_text_of_the_reply() {
    let bad = [
        tampered(|r| r.ingredients[0].text = "rapeseed oil".into()),
        tampered(|r| r.ingredients[0].qty = Some("2 tbsp".into())),
        tampered(|r| r.ingredients[0].group = Some("For the sauce".into())),
        tampered(|r| r.serves = Some(9)),
        tampered(|r| r.times[0].text = "20 min".into()),
        tampered(|r| r.steps[3].text = "Serve hot.".into()),
        tampered(|r| r.notes[0] = "Freezes well.".into()),
        tampered(|r| r.title = Some("Pumpkin stew".into())),
    ];
    for (k, set) in bad.iter().enumerate() {
        assert!(validate(set, Some(CONTRACT)).is_err(), "case {k}");
    }
}

#[test]
fn a_section_source_is_for_a_recipe_only() {
    let steps = "Steps:\n\n1. Mix.\n2. Bake.\n3. Cool.\n";
    let mut set = compute(steps).expect("steps");
    let Block::Steps(s) = &mut set.blocks[0] else { panic!() };
    s.source = Source::section((0, true), (0, true), 0);
    assert!(validate(&set, Some(steps)).is_err());
    // A list source can't carry section fields either
    let mut set = compute(steps).expect("steps");
    let Block::Steps(s) = &mut set.blocks[0] else { panic!() };
    s.source.lead = Some(0);
    assert!(validate(&set, Some(steps)).is_err());
}

#[test]
fn optional_fields_may_be_missing_when_read_back() {
    let v = json!({"version": 1, "blocks": [{
        "type": "recipe", "id": "b0",
        "source": {"kind": "markdown_section", "index": 0, "ordered": false, "last_index": 0, "last_ordered": true, "lead": 0},
        "ingredients": [{"id": "i0", "text": "eggs"}, {"id": "i1", "text": "milk"}],
        "steps": [{"id": "s0", "text": "Whisk."}, {"id": "s1", "text": "Bake."}],
        "provenance": {"kind": "reply_text"}
    }]});
    let set: ResultSet = serde_json::from_value(v).unwrap();
    let Block::Recipe(r) = &set.blocks[0] else { panic!() };
    assert_eq!((r.title.as_deref(), r.serves, r.times.len(), r.notes.len(), r.ingredients[0].qty.as_deref()), (None, None, 0, 0, None));
    assert_eq!(set.version, VERSION);
    assert!(validate(&set, None).is_ok());
}
