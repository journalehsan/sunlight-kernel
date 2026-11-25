__attribute__((noinline, noclone))
int helios_graph_multiply(int value)
{
    return value * 4;
}

int helios_graph_b(int value)
{
    return helios_graph_multiply(value);
}
