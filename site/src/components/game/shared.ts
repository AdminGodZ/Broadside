// What the page's markup (Game.astro, drawn at build time) and the script
// (game.ts) both need, so the first frame already looks like the game.

export const SHIPS: [string, number][] = [["Carrier", 5], ["Battleship", 4], ["Cruiser", 3], ["Submarine", 3], ["Destroyer", 2]];
export const ROWS = "ABCDEFGHIJ";
export const cellName = (c: number) => ROWS[Math.floor(c / 10)] + (c % 10 + 1);

export const TEXT = {
  ready: "Your fleet is ready",
  saved: "Your last fleet is in position. Move any ship, or start the battle.",
  random: "A random fleet. Move any ship, or start the battle.",
  moveOn: "Move any ship, or start the battle.",
  pickHint: "Click a ship on the chart, or in this list, to move it.",
};
