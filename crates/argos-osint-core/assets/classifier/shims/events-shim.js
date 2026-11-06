'use strict';

class EventEmitter {
  constructor() {
    this.count = 0;
  }
  emit(name, data) {
    this.count++;
  }
  on() {}
  once() {}
  removeListener() {}
}

module.exports = EventEmitter;
