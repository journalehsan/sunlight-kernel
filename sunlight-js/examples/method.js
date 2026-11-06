const object = { value: 41, next() { this.value = this.value + 1; return this.value; } };
print(object.next());
