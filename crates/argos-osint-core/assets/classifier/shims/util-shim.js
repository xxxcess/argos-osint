'use strict';

function inherits(child, parent) {
  child.prototype = Object.create(parent.prototype);
  child.prototype.constructor = child;
}

module.exports = {
  inherits: inherits
};
