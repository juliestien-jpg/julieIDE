<?php
namespace App\Model;

class Admin extends User
{
    public function getName(): string
    {
        return 'admin:' . parent::getName();
    }
}
