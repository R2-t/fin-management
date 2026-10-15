# Historical amounts keep their point-in-time currency

Purchase facts record the conversion that applied when the purchase happened, and changing a
wallet's or user's currency later does not restate those stored values. We chose this because a
user reviewing last March's spending must see the number they saw then; restating history against
a rate that did not exist at the time would silently rewrite reported figures. Filling in a
conversion that was *missing* entirely (an unresolved rate) is completion of an incomplete record,
not restatement, and is permitted.
