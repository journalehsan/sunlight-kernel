// Deliberately small Event subset. Native dispatch owns target/currentTarget.
function Event(type, options) {
    const event = Object.create(Event.prototype);
    event.type = type;
    event.bubbles = options ? options.bubbles === true : false;
    event.cancelable = options ? options.cancelable === true : false;
    event.defaultPrevented = false;
    event.target = null;
    event.currentTarget = null;
    event._stop = false;
    event._dispatching = false;
    return event;
}
Event.prototype.preventDefault = function() {
    if (this.cancelable) this.defaultPrevented = true;
};
Event.prototype.stopPropagation = function() { this._stop = true; };
