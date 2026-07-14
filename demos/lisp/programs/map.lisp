; map, written in Lisp itself out of cons/car/cdr
(define (map f xs)
  (if (null? xs)
      '()
      (cons (f (car xs)) (map f (cdr xs)))))

(define (square x) (* x x))

(map square (list 1 2 3 4 5))
(map (lambda (x) (* 2 x)) '(10 20 30))
