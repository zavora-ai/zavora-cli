from calculator import calculator


def test_add():
    assert calculator.add(1, 2) == 3


def test_div_zero():
    try:
        calculator.divide(1, 0)
    except ZeroDivisionError:
        assert True
    else:
        assert False


def test_mean():
    assert calculator.mean([1, 2, 3]) == 2
