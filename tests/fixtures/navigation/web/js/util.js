export function formatDate(d) { return d.toISOString(); }

export default class Api { get(url) { return fetch(url); } }
