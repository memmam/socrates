; factorial -- the classic recursion demo
(define (fact n)
  (if (< n 2)
      1
      (* n (fact (- n 1)))))

(fact 10)
(fact 20)
