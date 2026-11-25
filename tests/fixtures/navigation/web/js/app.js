import Api, { formatDate as fd } from './util';
import { formatDate } from './index.js';

const api = new Api();
api.get('/x');
fd(new Date());
formatDate(new Date());
function local() { return 1; }
local();
