#include "io/SourceClaimBudget.h"
#include <limits>

int main() {
    if (!onecad::io::source_capacity_fits(10, 20) ||
        onecad::io::source_capacity_fits(10, 21) ||
        onecad::io::source_capacity_fits(std::numeric_limits<std::size_t>::max(), 1)) return 6;
    onecad::io::SourceClaimBudget budget(10);
    auto first = budget.acquire(6);
    if (!first || budget.used() != 6 || budget.acquire(5) ||
        budget.acquire(std::numeric_limits<std::size_t>::max())) return 1;
    auto checkpoint = first;
    first.reset();
    if (budget.used() != 6 || budget.acquire(5)) return 2;
    auto second = budget.acquire(4);
    if (!second || budget.used() != 10) return 3;
    checkpoint.reset();
    if (budget.used() != 4) return 4;
    second.reset();
    return budget.used() == 0 ? 0 : 5;
}
